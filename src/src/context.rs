use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use once_cell::sync::OnceCell;
use rustpython_parser::{
    Parse,
    ast::{self, Ranged},
};
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};

use crate::{
    error::{AppError, Result},
    project_config,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDef {
    pub path: PathBuf,
    pub language: Language,
    pub imports: Vec<Import>,
    pub defs: Vec<Definition>,
    pub source_lines: Vec<String>,
    pub declared_namespaces: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    Rust,
    #[serde(rename = "csharp")]
    CSharp,
    TypeScript,
    JavaScript,
}

impl Language {
    pub const ALL: [Self; 5] = [
        Self::Python,
        Self::Rust,
        Self::CSharp,
        Self::TypeScript,
        Self::JavaScript,
    ];

    pub fn from_path(path: &Path) -> Option<Self> {
        let path = path.to_string_lossy().to_ascii_lowercase();
        if path.ends_with(".tsx") || path.ends_with(".ts") {
            Some(Self::TypeScript)
        } else if path.ends_with(".jsx")
            || path.ends_with(".mjs")
            || path.ends_with(".cjs")
            || path.ends_with(".js")
        {
            Some(Self::JavaScript)
        } else if path.ends_with(".py") {
            Some(Self::Python)
        } else if path.ends_with(".rs") {
            Some(Self::Rust)
        } else if path.ends_with(".cs") {
            Some(Self::CSharp)
        } else {
            None
        }
    }

    pub fn config_name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Rust => "rust",
            Self::CSharp => "csharp",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
        }
    }

    pub fn from_config_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "python" | "py" => Some(Self::Python),
            "rust" | "rs" => Some(Self::Rust),
            "csharp" | "c#" | "cs" => Some(Self::CSharp),
            "typescript" | "ts" | "tsx" => Some(Self::TypeScript),
            "javascript" | "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            _ => None,
        }
    }

    pub fn order(self) -> usize {
        match self {
            Self::Python => 0,
            Self::Rust => 1,
            Self::CSharp => 2,
            Self::TypeScript => 3,
            Self::JavaScript => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub module: String,
    pub name: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub name: String,
    pub kind: DefKind,
    pub line: usize,
    pub end_line: usize,
    pub has_args: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
        for (path, _) in supported_source_files(&self.path)? {
            let rel = rel_string(&self.path, &path)?;
            modules.push(self.get_or_parse(rel)?);
        }
        if modules.is_empty() {
            return Err(AppError::NoSource(self.path.clone()));
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
        let language = Language::from_path(&full_path).ok_or_else(|| {
            AppError::Validation(format!("unsupported source file: {}", full_path.display()))
        })?;
        let source = std::fs::read_to_string(&full_path)?;
        let source_lines = source.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        let (imports, defs) = match language {
            Language::Python => parse_python_module(&full_path, &source)?,
            Language::Rust | Language::CSharp | Language::TypeScript | Language::JavaScript => {
                parse_tree_sitter_module(language, &full_path, &source)?
            }
        };
        let module = ModuleDef {
            path: PathBuf::from(&rel_path),
            language,
            imports,
            defs,
            source_lines,
            declared_namespaces: declared_namespaces(language, &source),
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

pub fn supported_source_files(root: &Path) -> Result<Vec<(PathBuf, Language)>> {
    enabled_source_files(root)
}

pub fn all_supported_source_files(root: &Path) -> Result<Vec<(PathBuf, Language)>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if is_ignored_path(path) {
            continue;
        }
        if let Some(language) = Language::from_path(path) {
            files.push((path.to_path_buf(), language));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

pub fn enabled_source_files(root: &Path) -> Result<Vec<(PathBuf, Language)>> {
    let enabled = project_config::enabled_languages(root)?;
    Ok(all_supported_source_files(root)?
        .into_iter()
        .filter(|(_, language)| enabled.contains(language))
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageCount {
    pub language: Language,
    pub count: usize,
}

pub fn detect_languages(root: &Path) -> Result<Vec<LanguageCount>> {
    let mut counts = HashMap::<Language, usize>::new();
    for (_, language) in all_supported_source_files(root)? {
        *counts.entry(language).or_default() += 1;
    }
    let mut rows = counts
        .into_iter()
        .map(|(language, count)| LanguageCount { language, count })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.language.order());
    Ok(rows)
}

pub fn python_files(root: &Path) -> Result<Vec<PathBuf>> {
    Ok(supported_source_files(root)?
        .into_iter()
        .filter_map(|(path, language)| (language == Language::Python).then_some(path))
        .collect())
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
    if parts
        .last()
        .is_some_and(|part| matches!(part.as_str(), "__init__" | "mod" | "index"))
    {
        parts.pop();
    }
    parts.join(".")
}

#[derive(Debug, Clone)]
pub struct LocalModuleResolver {
    module_names: Vec<String>,
    module_to_index: HashMap<String, usize>,
    path_to_module: HashMap<PathBuf, String>,
    module_by_path: HashMap<PathBuf, String>,
    aliases: HashMap<String, Vec<String>>,
}

impl LocalModuleResolver {
    pub fn new(modules: &[ModuleDef]) -> Self {
        let mut resolver = Self {
            module_names: Vec::new(),
            module_to_index: HashMap::new(),
            path_to_module: HashMap::new(),
            module_by_path: HashMap::new(),
            aliases: HashMap::new(),
        };

        for module in modules {
            let name = module_name_from_path(&module.path);
            resolver.insert_module_name(name.clone());
            resolver
                .path_to_module
                .insert(normalize_path(&module.path), name.clone());
            resolver
                .module_by_path
                .insert(module.path.clone(), name.clone());
            resolver.add_alias(name.clone(), name.clone());
            for alias in module_aliases(module, &name) {
                resolver.add_alias(alias, name.clone());
            }
        }

        for modules in resolver.aliases.values_mut() {
            modules.sort();
            modules.dedup();
        }

        resolver
    }

    pub fn module_names(&self) -> &[String] {
        &self.module_names
    }

    pub fn module_index(&self, module: &str) -> Option<usize> {
        self.module_to_index.get(module).copied()
    }

    pub fn module_name_for(&self, module: &ModuleDef) -> String {
        self.module_by_path
            .get(&module.path)
            .cloned()
            .unwrap_or_else(|| module_name_from_path(&module.path))
    }

    pub fn resolve_import_targets(&self, source: &ModuleDef, import: &Import) -> Vec<String> {
        let mut targets = match source.language {
            Language::Python => self.resolve_python_import(source, import),
            Language::Rust => self.resolve_rust_import(source, import),
            Language::CSharp => self.resolve_by_candidate(&import.module),
            Language::TypeScript | Language::JavaScript => {
                self.resolve_js_ts_import(source, import)
            }
        };
        targets.sort();
        targets.dedup();
        targets
    }

    pub fn import_matches_target(
        &self,
        source: &ModuleDef,
        import: &Import,
        target_module: &str,
    ) -> bool {
        if raw_import_matches(import, target_module) {
            return true;
        }

        let target_candidates = self.target_candidates(target_module);
        self.resolve_import_targets(source, import)
            .iter()
            .any(|target| target_candidates.contains(target))
    }

    fn insert_module_name(&mut self, module: String) {
        if self.module_to_index.contains_key(&module) {
            return;
        }
        let idx = self.module_names.len();
        self.module_to_index.insert(module.clone(), idx);
        self.module_names.push(module);
    }

    fn add_alias(&mut self, alias: String, module: String) {
        if alias.is_empty() {
            return;
        }
        self.aliases.entry(alias).or_default().push(module);
    }

    fn target_candidates(&self, target: &str) -> HashSet<String> {
        let mut candidates = HashSet::from([target.to_string()]);
        if let Some(targets) = self.aliases.get(target) {
            candidates.extend(targets.iter().cloned());
        }
        candidates
    }

    fn resolve_python_import(&self, source: &ModuleDef, import: &Import) -> Vec<String> {
        let raw = import.module.as_str();
        if raw.starts_with('.') {
            let level = raw.chars().take_while(|ch| *ch == '.').count();
            let tail = raw.trim_start_matches('.');
            let mut base = python_package_parts(&self.module_name_for(source), &source.path);
            for _ in 1..level {
                base.pop();
            }

            let mut candidates = Vec::new();
            if tail.is_empty() {
                if let Some(name) = import.name.as_deref() {
                    let mut named = base.clone();
                    named.push(name.to_string());
                    candidates.push(named.join("."));
                }
                if !base.is_empty() {
                    candidates.push(base.join("."));
                }
            } else {
                let mut module = base;
                module.extend(tail.split('.').map(ToOwned::to_owned));
                candidates.push(module.join("."));
                if let Some(name) = import.name.as_deref() {
                    candidates.push(format!("{}.{}", candidates[0], name));
                }
            }
            return self.resolve_first_candidate(candidates);
        }

        let mut candidates = vec![raw.to_string()];
        if let Some(name) = import.name.as_deref() {
            candidates.push(format!("{raw}.{name}"));
        }
        self.resolve_first_candidate(candidates)
    }

    fn resolve_rust_import(&self, source: &ModuleDef, import: &Import) -> Vec<String> {
        let parts = import
            .module
            .split('.')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            return Vec::new();
        }

        let source_name = self.module_name_for(source);
        let candidate = match parts[0] {
            "crate" => join_prefix_and_tail(vec!["src".to_string()], &parts[1..]),
            "self" => join_prefix_and_tail(split_module(&source_name), &parts[1..]),
            "super" => {
                let mut base = split_module(&source_name);
                let mut idx = 0;
                while idx < parts.len() && parts[idx] == "super" {
                    base.pop();
                    idx += 1;
                }
                join_prefix_and_tail(base, &parts[idx..])
            }
            _ => import.module.clone(),
        };

        self.resolve_by_candidate(&candidate)
    }

    fn resolve_js_ts_import(&self, source: &ModuleDef, import: &Import) -> Vec<String> {
        let raw = import.module.split(['?', '#']).next().unwrap_or_default();
        if raw.starts_with('.') || raw.starts_with('/') {
            let base = if raw.starts_with('/') {
                PathBuf::new()
            } else {
                source
                    .path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default()
            };
            let candidate = normalize_path(&base.join(raw));
            return self.resolve_path_candidate(&candidate);
        }

        self.resolve_by_candidate(&raw.replace('/', "."))
    }

    fn resolve_path_candidate(&self, candidate: &Path) -> Vec<String> {
        let mut paths = Vec::new();
        if candidate.extension().is_some() {
            paths.push(normalize_path(candidate));
        } else {
            for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
                paths.push(normalize_path(&candidate.with_extension(ext)));
            }
            for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs"] {
                paths.push(normalize_path(&candidate.join("index").with_extension(ext)));
            }
        }

        let mut targets = paths
            .iter()
            .filter_map(|path| self.path_to_module.get(path).cloned())
            .collect::<Vec<_>>();
        targets.sort();
        targets.dedup();
        targets
    }

    fn resolve_first_candidate(&self, candidates: Vec<String>) -> Vec<String> {
        candidates
            .into_iter()
            .find_map(|candidate| {
                let targets = self.resolve_by_candidate(&candidate);
                (!targets.is_empty()).then_some(targets)
            })
            .unwrap_or_default()
    }

    fn resolve_by_candidate(&self, candidate: &str) -> Vec<String> {
        let mut parts = candidate
            .split('.')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        while !parts.is_empty() {
            let key = parts.join(".");
            if let Some(targets) = self.aliases.get(&key) {
                return targets.clone();
            }
            parts.pop();
        }
        Vec::new()
    }
}

fn module_aliases(module: &ModuleDef, module_name: &str) -> Vec<String> {
    match module.language {
        Language::Rust => rust_module_aliases(module_name),
        Language::CSharp => csharp_module_aliases(module, module_name),
        _ => Vec::new(),
    }
}

fn rust_module_aliases(module_name: &str) -> Vec<String> {
    let Some(rest) = module_name.strip_prefix("src.") else {
        return Vec::new();
    };
    vec![rest.to_string(), format!("crate.{rest}")]
}

fn csharp_module_aliases(module: &ModuleDef, module_name: &str) -> Vec<String> {
    let namespaces = module.declared_namespaces.clone();
    let mut aliases = Vec::new();
    for namespace in namespaces {
        aliases.push(namespace.clone());
        for def in &module.defs {
            aliases.push(format!("{namespace}.{}", def.name));
        }
    }
    aliases.push(module_name.to_string());
    aliases
}

fn declared_namespaces(language: Language, source: &str) -> Vec<String> {
    if language != Language::CSharp {
        return Vec::new();
    }
    let mut namespaces = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("namespace ") else {
            continue;
        };
        let namespace = rest.split([';', '{']).next().unwrap_or_default().trim();
        if !namespace.is_empty() {
            namespaces.push(namespace.to_string());
        }
    }
    namespaces.sort();
    namespaces.dedup();
    namespaces
}

fn python_package_parts(module_name: &str, path: &Path) -> Vec<String> {
    let mut parts = split_module(module_name);
    if path
        .file_stem()
        .is_none_or(|stem| stem.to_string_lossy() != "__init__")
    {
        parts.pop();
    }
    parts
}

fn split_module(module: &str) -> Vec<String> {
    module
        .split('.')
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn join_prefix_and_tail(prefix: Vec<String>, tail: &[&str]) -> String {
    prefix
        .into_iter()
        .chain(tail.iter().map(|part| (*part).to_string()))
        .collect::<Vec<_>>()
        .join(".")
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }
    normalized
}

fn raw_import_matches(import: &Import, target_module: &str) -> bool {
    if import.module == target_module || import.module.starts_with(&format!("{target_module}.")) {
        return true;
    }
    if let Some((parent, name)) = target_module.rsplit_once('.') {
        return import.module == parent && import.name.as_deref() == Some(name);
    }
    import.name.as_deref() == Some(target_module)
}

fn is_ignored_path(path: &Path) -> bool {
    path.components().any(|part| {
        matches!(
            part.as_os_str().to_string_lossy().as_ref(),
            ".git" | ".rustrank" | "target" | "node_modules" | "dist" | "build"
        )
    })
}

fn parse_python_module(path: &Path, source: &str) -> Result<(Vec<Import>, Vec<Definition>)> {
    match ast::Suite::parse(source, &path.to_string_lossy()) {
        Ok(suite) => Ok((
            extract_imports_from_suite(&suite, source),
            extract_defs_from_suite(&suite, source),
        )),
        Err(source_err) => {
            let (imports, defs) = parse_python_with_tree_sitter(path, source)
                .unwrap_or_else(|_| lazy_parse_python_module(source));
            if imports.is_empty() && defs.is_empty() {
                Err(AppError::Parse {
                    path: path.to_path_buf(),
                    source: source_err,
                })
            } else {
                Ok((imports, defs))
            }
        }
    }
}

fn parse_python_with_tree_sitter(
    path: &Path,
    source: &str,
) -> Result<(Vec<Import>, Vec<Definition>)> {
    let mut parser = Parser::new();
    let parser_language = tree_sitter_language(Language::Python, path).ok_or_else(|| {
        AppError::Validation(format!(
            "unsupported Tree-sitter language: {:?}",
            Language::Python
        ))
    })?;
    parser
        .set_language(&parser_language)
        .map_err(|err| AppError::Context(err.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| AppError::Context(format!("failed to parse {}", path.display())))?;

    let mut imports = Vec::new();
    let mut defs = Vec::new();
    collect_tree_sitter(
        tree.root_node(),
        Language::Python,
        source,
        &mut imports,
        &mut defs,
    );
    normalize_python_facts(&mut imports, &mut defs);
    if imports.is_empty() && defs.is_empty() {
        Ok(lazy_parse_python_module(source))
    } else {
        Ok((imports, defs))
    }
}

fn lazy_parse_python_module(source: &str) -> (Vec<Import>, Vec<Definition>) {
    let mut imports = Vec::new();
    let mut defs = Vec::new();
    for (idx, line) in source.lines().enumerate() {
        let line_no = idx + 1;
        collect_lazy_python_imports(line, line_no, &mut imports);
        if let Some(def) = lazy_python_def(line, line_no) {
            defs.push(def);
        }
    }
    normalize_python_facts(&mut imports, &mut defs);
    (imports, defs)
}

fn normalize_python_facts(imports: &mut Vec<Import>, defs: &mut Vec<Definition>) {
    imports.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.module.cmp(&b.module))
            .then_with(|| a.name.cmp(&b.name))
    });
    imports.dedup_by(|a, b| a.line == b.line && a.module == b.module && a.name == b.name);
    defs.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.name.cmp(&b.name)));
    defs.dedup_by(|a, b| a.line == b.line && a.name == b.name && a.kind == b.kind);
}

fn collect_lazy_python_imports(line: &str, line_no: usize, imports: &mut Vec<Import>) {
    let trimmed = strip_python_comment(line).trim();
    if let Some(rest) = trimmed.strip_prefix("import ") {
        for module in rest.split(',').filter_map(python_import_name) {
            imports.push(Import {
                module,
                name: None,
                line: line_no,
            });
        }
        return;
    }

    let Some(rest) = trimmed.strip_prefix("from ") else {
        return;
    };
    let Some((module, names)) = rest.split_once(" import ") else {
        return;
    };
    let module = module.trim();
    if module.is_empty() {
        return;
    }
    for name in names.split(',').filter_map(python_import_name) {
        imports.push(Import {
            module: module.to_string(),
            name: Some(name),
            line: line_no,
        });
    }
}

fn lazy_python_def(line: &str, line_no: usize) -> Option<Definition> {
    let trimmed = strip_python_comment(line).trim();
    let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("async def ") {
        (DefKind::Func, rest)
    } else if let Some(rest) = trimmed.strip_prefix("def ") {
        (DefKind::Func, rest)
    } else if let Some(rest) = trimmed.strip_prefix("class ") {
        let name_end = rest.find(['(', ':']).unwrap_or(rest.len());
        let kind = if rest[name_end..].trim_start().starts_with('(') {
            DefKind::Struct
        } else {
            DefKind::Class
        };
        (kind, rest)
    } else {
        return None;
    };

    let name_end = rest.find(['(', ':']).unwrap_or(rest.len());
    let name = rest[..name_end].trim();
    if name.is_empty() {
        return None;
    }
    Some(Definition {
        name: name.to_string(),
        kind,
        line: line_no,
        end_line: line_no,
        has_args: rest[name_end..].trim_start().starts_with('('),
    })
}

fn python_import_name(value: &str) -> Option<String> {
    let name = value.trim().split(" as ").next().unwrap_or_default().trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn strip_python_comment(line: &str) -> &str {
    line.split_once('#')
        .map(|(before, _)| before)
        .unwrap_or(line)
}

fn parse_tree_sitter_module(
    language: Language,
    path: &Path,
    source: &str,
) -> Result<(Vec<Import>, Vec<Definition>)> {
    let mut parser = Parser::new();
    let parser_language = tree_sitter_language(language, path).ok_or_else(|| {
        AppError::Validation(format!("unsupported Tree-sitter language: {language:?}"))
    })?;
    parser
        .set_language(&parser_language)
        .map_err(|err| AppError::Context(err.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| AppError::Context(format!("failed to parse {}", path.display())))?;
    let mut imports = Vec::new();
    let mut defs = Vec::new();
    collect_tree_sitter(tree.root_node(), language, source, &mut imports, &mut defs);
    imports.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
            .then_with(|| a.module.cmp(&b.module))
            .then_with(|| a.name.cmp(&b.name))
    });
    imports.dedup_by(|a, b| a.line == b.line && a.module == b.module && a.name == b.name);
    defs.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.name.cmp(&b.name)));
    defs.dedup_by(|a, b| a.line == b.line && a.name == b.name && a.kind == b.kind);
    Ok((imports, defs))
}

fn tree_sitter_language(language: Language, path: &Path) -> Option<tree_sitter::Language> {
    match language {
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        Language::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
        Language::TypeScript
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("tsx")) =>
        {
            Some(tree_sitter_typescript::LANGUAGE_TSX.into())
        }
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::JavaScript => Some(tree_sitter_javascript::LANGUAGE.into()),
    }
}

fn collect_tree_sitter(
    node: Node<'_>,
    language: Language,
    source: &str,
    imports: &mut Vec<Import>,
    defs: &mut Vec<Definition>,
) {
    match language {
        Language::Python => collect_python_node(node, source, imports, defs),
        Language::Rust => collect_rust_node(node, source, imports, defs),
        Language::CSharp => collect_csharp_node(node, source, imports, defs),
        Language::TypeScript | Language::JavaScript => {
            collect_js_ts_node(node, language, source, imports, defs);
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_tree_sitter(child, language, source, imports, defs);
    }
}

fn collect_python_node(
    node: Node<'_>,
    source: &str,
    imports: &mut Vec<Import>,
    defs: &mut Vec<Definition>,
) {
    match node.kind() {
        "import_statement" | "import_from_statement" => {
            collect_lazy_python_imports(
                node_text(node, source),
                start_line_for_node(node),
                imports,
            );
        }
        "function_definition" => push_tree_sitter_def(node, source, DefKind::Func, defs),
        "class_definition" => {
            let kind = if node_text(node, source).contains('(') {
                DefKind::Struct
            } else {
                DefKind::Class
            };
            push_tree_sitter_def(node, source, kind, defs);
        }
        _ => {}
    }
}

fn collect_rust_node(
    node: Node<'_>,
    source: &str,
    imports: &mut Vec<Import>,
    defs: &mut Vec<Definition>,
) {
    match node.kind() {
        "use_declaration" => {
            if let Some(module) = rust_import_from_text(node_text(node, source)) {
                imports.push(Import {
                    module,
                    name: None,
                    line: start_line_for_node(node),
                });
            }
        }
        "function_item" => push_tree_sitter_def(node, source, DefKind::Func, defs),
        "struct_item" => push_tree_sitter_def(node, source, DefKind::Struct, defs),
        "enum_item" => push_tree_sitter_def(node, source, DefKind::Enum, defs),
        "trait_item" => push_tree_sitter_def(node, source, DefKind::Trait, defs),
        _ => {}
    }
}

fn collect_csharp_node(
    node: Node<'_>,
    source: &str,
    imports: &mut Vec<Import>,
    defs: &mut Vec<Definition>,
) {
    match node.kind() {
        "using_directive" => {
            if let Some(module) = csharp_import_from_text(node_text(node, source)) {
                imports.push(Import {
                    module,
                    name: None,
                    line: start_line_for_node(node),
                });
            }
        }
        "class_declaration" => push_tree_sitter_def(node, source, DefKind::Class, defs),
        "record_declaration" | "struct_declaration" => {
            push_tree_sitter_def(node, source, DefKind::Struct, defs);
        }
        "interface_declaration" => push_tree_sitter_def(node, source, DefKind::Trait, defs),
        "enum_declaration" => push_tree_sitter_def(node, source, DefKind::Enum, defs),
        "method_declaration" | "constructor_declaration" => {
            push_tree_sitter_def(node, source, DefKind::Func, defs);
        }
        _ => {}
    }
}

fn collect_js_ts_node(
    node: Node<'_>,
    language: Language,
    source: &str,
    imports: &mut Vec<Import>,
    defs: &mut Vec<Definition>,
) {
    match node.kind() {
        "import_statement" | "export_statement" => {
            if let Some(module) = quoted_module_from_text(node_text(node, source)) {
                imports.push(Import {
                    module,
                    name: None,
                    line: start_line_for_node(node),
                });
            }
        }
        "call_expression" => {
            let text = node_text(node, source);
            if text.contains("require(")
                && let Some(module) = quoted_module_from_text(text)
            {
                imports.push(Import {
                    module,
                    name: None,
                    line: start_line_for_node(node),
                });
            }
        }
        "function_declaration" | "generator_function_declaration" => {
            push_tree_sitter_def(node, source, DefKind::Func, defs);
        }
        "class_declaration" => push_tree_sitter_def(node, source, DefKind::Class, defs),
        "interface_declaration" if language == Language::TypeScript => {
            push_tree_sitter_def(node, source, DefKind::Trait, defs);
        }
        "enum_declaration" if language == Language::TypeScript => {
            push_tree_sitter_def(node, source, DefKind::Enum, defs);
        }
        "method_definition" => push_tree_sitter_def(node, source, DefKind::Func, defs),
        "variable_declarator" if variable_declarator_is_callable(node) => {
            push_tree_sitter_def(node, source, DefKind::Func, defs);
        }
        _ => {}
    }
}

fn push_tree_sitter_def(node: Node<'_>, source: &str, kind: DefKind, defs: &mut Vec<Definition>) {
    let Some(name) = name_for_node(node, source) else {
        return;
    };
    defs.push(Definition {
        name,
        kind,
        line: start_line_for_node(node),
        end_line: end_line_for_node(node),
        has_args: node_text(node, source).contains('('),
    });
}

fn name_for_node(node: Node<'_>, source: &str) -> Option<String> {
    let name = node.child_by_field_name("name")?;
    Some(node_text(name, source).trim().to_string()).filter(|name| !name.is_empty())
}

fn variable_declarator_is_callable(node: Node<'_>) -> bool {
    node.child_by_field_name("value").is_some_and(|value| {
        matches!(
            value.kind(),
            "arrow_function" | "function_expression" | "generator_function"
        )
    })
}

fn rust_import_from_text(text: &str) -> Option<String> {
    let mut text = text.trim();
    text = text
        .strip_prefix("pub use ")
        .or_else(|| text.strip_prefix("use "))
        .unwrap_or(text)
        .trim();
    text = text.trim_end_matches(';').trim();
    let text = text.split(" as ").next().unwrap_or(text);
    let text = text.split('{').next().unwrap_or(text).trim_end_matches(':');
    let module = text.replace("::", ".");
    let mut parts = module
        .split('.')
        .filter(|part| !part.is_empty() && *part != "self")
        .collect::<Vec<_>>();
    if parts.len() > 2 {
        parts.pop();
    }
    (!parts.is_empty()).then(|| parts.join("."))
}

fn csharp_import_from_text(text: &str) -> Option<String> {
    let mut text = text.trim();
    text = text.strip_prefix("using")?.trim();
    text = text.strip_prefix("static ").unwrap_or(text).trim();
    if let Some((_, module)) = text.split_once('=') {
        text = module.trim();
    }
    let module = text.trim_end_matches(';').trim();
    (!module.is_empty()).then(|| module.to_string())
}

fn quoted_module_from_text(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        let quote = bytes[idx];
        if matches!(quote, b'\'' | b'"' | b'`') {
            let start = idx + 1;
            let mut end = start;
            while end < bytes.len() {
                if bytes[end] == b'\\' {
                    end += 2;
                    continue;
                }
                if bytes[end] == quote {
                    return Some(text[start..end].to_string());
                }
                end += 1;
            }
            return None;
        }
        idx += 1;
    }
    None
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn start_line_for_node(node: Node<'_>) -> usize {
    node.start_position().row + 1
}

fn end_line_for_node(node: Node<'_>) -> usize {
    node.end_position().row + 1
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
            let level = node.level.map(|level| level.to_usize()).unwrap_or(0);
            let module = format!(
                "{}{}",
                ".".repeat(level),
                node.module
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            );
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
