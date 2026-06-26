use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use once_cell::sync::OnceCell;
use rustpython_parser::{
    Parse,
    ast::{self, Ranged},
};

use crate::error::{AppError, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDef {
    pub path: PathBuf,
    pub imports: Vec<Import>,
    pub defs: Vec<Definition>,
    pub source_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub module: String,
    pub name: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub name: String,
    pub kind: DefKind,
    pub line: usize,
    pub end_line: usize,
    pub has_args: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefKind {
    Func,
    Struct,
    Class,
    Trait,
    Enum,
}

pub struct Context {
    path: PathBuf,
    pub(crate) parse_cache: Arc<Mutex<HashMap<String, ModuleDef>>>,
    git_repo: OnceCell<RepositoryHolder>,
}

struct RepositoryHolder(git2::Repository);

impl Context {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            parse_cache: Arc::new(Mutex::new(HashMap::new())),
            git_repo: OnceCell::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cache_len(&self) -> usize {
        self.parse_cache
            .lock()
            .map(|cache| cache.len())
            .unwrap_or(0)
    }

    pub fn parse_all(&self) -> Result<Vec<ModuleDef>> {
        let mut modules = Vec::new();
        for path in python_files(&self.path)? {
            let rel = rel_string(&self.path, &path)?;
            modules.push(self.get_or_parse(rel)?);
        }
        if modules.is_empty() {
            return Err(AppError::NoPythonSource(self.path.clone()));
        }
        modules.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(modules)
    }

    pub fn get_or_parse(&self, rel_path: String) -> Result<ModuleDef> {
        if let Some(cached) = self
            .parse_cache
            .lock()
            .map_err(|err| AppError::Context(err.to_string()))?
            .get(&rel_path)
            .cloned()
        {
            return Ok(cached);
        }

        let full_path = self.path.join(&rel_path);
        let source = std::fs::read_to_string(&full_path)?;
        let suite = ast::Suite::parse(&source, &full_path.to_string_lossy()).map_err(|source| {
            AppError::Parse {
                path: full_path.clone(),
                source,
            }
        })?;

        let source_lines = source.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        let imports = extract_imports_from_suite(&suite, &source);
        let defs = extract_defs_from_suite(&suite, &source);
        let module = ModuleDef {
            path: PathBuf::from(&rel_path),
            imports,
            defs,
            source_lines,
        };

        self.parse_cache
            .lock()
            .map_err(|err| AppError::Context(err.to_string()))?
            .insert(rel_path, module.clone());
        Ok(module)
    }

    pub fn git_repo(&self) -> Result<&git2::Repository> {
        self.git_repo
            .get_or_try_init(|| {
                git2::Repository::discover(&self.path)
                    .map(RepositoryHolder)
                    .map_err(|_| AppError::NotAGit(self.path.clone()))
            })
            .map(|holder| &holder.0)
    }
}

pub fn python_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.components().any(|part| part.as_os_str() == ".git") {
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "py") {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    Ok(files)
}

pub fn rel_string(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|err| AppError::Context(err.to_string()))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

pub fn module_name_from_path(path: &Path) -> String {
    let mut parts = path
        .with_extension("")
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if parts.last().is_some_and(|part| part == "__init__") {
        parts.pop();
    }
    parts.join(".")
}

fn extract_imports_from_suite(suite: &ast::Suite, source: &str) -> Vec<Import> {
    let mut imports = Vec::new();
    for stmt in suite {
        collect_imports(stmt, source, &mut imports);
    }
    imports.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.module.cmp(&b.module))
            .then_with(|| a.name.cmp(&b.name))
    });
    imports
}

fn collect_imports(stmt: &ast::Stmt, source: &str, imports: &mut Vec<Import>) {
    let line = start_line(source, stmt);
    match stmt {
        ast::Stmt::Import(node) => {
            for alias in &node.names {
                imports.push(Import {
                    module: alias.name.to_string(),
                    name: None,
                    line,
                });
            }
        }
        ast::Stmt::ImportFrom(node) => {
            let module = node
                .module
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            for alias in &node.names {
                imports.push(Import {
                    module: module.clone(),
                    name: Some(alias.name.to_string()),
                    line,
                });
            }
        }
        _ => {}
    }

    for block in child_blocks(stmt) {
        for child in block {
            collect_imports(child, source, imports);
        }
    }
}

fn extract_defs_from_suite(suite: &ast::Suite, source: &str) -> Vec<Definition> {
    let mut defs = Vec::new();
    for stmt in suite {
        collect_defs(stmt, source, &mut defs);
    }
    defs.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.name.cmp(&b.name)));
    defs
}

fn collect_defs(stmt: &ast::Stmt, source: &str, defs: &mut Vec<Definition>) {
    let (line, end_line) = range_lines(source, stmt);
    match stmt {
        ast::Stmt::FunctionDef(node) => defs.push(Definition {
            name: node.name.to_string(),
            kind: DefKind::Func,
            line,
            end_line,
            has_args: has_function_args(&node.args),
        }),
        ast::Stmt::AsyncFunctionDef(node) => defs.push(Definition {
            name: node.name.to_string(),
            kind: DefKind::Func,
            line,
            end_line,
            has_args: has_function_args(&node.args),
        }),
        ast::Stmt::ClassDef(node) => defs.push(Definition {
            name: node.name.to_string(),
            kind: if node.bases.is_empty() && node.keywords.is_empty() {
                DefKind::Class
            } else {
                DefKind::Struct
            },
            line,
            end_line,
            has_args: !node.bases.is_empty() || !node.keywords.is_empty(),
        }),
        _ => {}
    }

    for block in child_blocks(stmt) {
        for child in block {
            collect_defs(child, source, defs);
        }
    }
}

fn child_blocks(stmt: &ast::Stmt) -> Vec<&[ast::Stmt]> {
    match stmt {
        ast::Stmt::FunctionDef(node) => vec![&node.body],
        ast::Stmt::AsyncFunctionDef(node) => vec![&node.body],
        ast::Stmt::ClassDef(node) => vec![&node.body],
        ast::Stmt::For(node) => vec![&node.body, &node.orelse],
        ast::Stmt::AsyncFor(node) => vec![&node.body, &node.orelse],
        ast::Stmt::While(node) => vec![&node.body, &node.orelse],
        ast::Stmt::If(node) => vec![&node.body, &node.orelse],
        ast::Stmt::With(node) => vec![&node.body],
        ast::Stmt::AsyncWith(node) => vec![&node.body],
        ast::Stmt::Try(node) => {
            let mut blocks = vec![
                node.body.as_slice(),
                node.orelse.as_slice(),
                node.finalbody.as_slice(),
            ];
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                blocks.push(handler.body.as_slice());
            }
            blocks
        }
        ast::Stmt::TryStar(node) => {
            let mut blocks = vec![
                node.body.as_slice(),
                node.orelse.as_slice(),
                node.finalbody.as_slice(),
            ];
            for handler in &node.handlers {
                let ast::ExceptHandler::ExceptHandler(handler) = handler;
                blocks.push(handler.body.as_slice());
            }
            blocks
        }
        _ => Vec::new(),
    }
}

fn has_function_args(args: &ast::Arguments) -> bool {
    !args.posonlyargs.is_empty()
        || !args.args.is_empty()
        || args.vararg.is_some()
        || !args.kwonlyargs.is_empty()
        || args.kwarg.is_some()
}

fn start_line(source: &str, stmt: &ast::Stmt) -> usize {
    line_for_offset(source, usize::from(stmt.start()))
}

fn range_lines(source: &str, stmt: &ast::Stmt) -> (usize, usize) {
    let start = usize::from(stmt.start());
    let end = usize::from(stmt.end());
    let last_byte = end.saturating_sub(1).min(source.len());
    (
        line_for_offset(source, start),
        line_for_offset(source, last_byte),
    )
}

fn line_for_offset(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

pub fn indentation(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}
