use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use rmcp::model::{Annotated, RawResource, RawResourceTemplate, Resource, ResourceTemplate};
use serde::{Deserialize, Serialize};

use crate::{
    context::{Context, Definition, Import, LocalModuleResolver, ModuleDef},
    embeddings::{self, EmbeddingOptions},
    error::{AppError, Result},
    process::{derive_processes, process_for_symbol},
};

use super::search::validate_dir;

const CONTEXT_URI: &str = "rustrank://repo/current/context";
const SCHEMA_URI: &str = "rustrank://repo/current/schema";
const MODULES_URI: &str = "rustrank://repo/current/modules";
const PROCESSES_URI: &str = "rustrank://repo/current/processes";
const MODULE_URI_PREFIX: &str = "rustrank://repo/current/module/";
const PROCESS_URI_PREFIX: &str = "rustrank://repo/current/process/";
static CURRENT_REPO: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolContext {
    pub symbol: String,
    pub defining_file: String,
    pub defining_module: String,
    pub kind: String,
    pub line: usize,
    pub end_line: usize,
    pub callers: Vec<SymbolRelation>,
    pub callees: Vec<SymbolRelation>,
    pub related_imports: Vec<Import>,
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolRelation {
    pub symbol: String,
    pub file: String,
    pub module: String,
    pub line: usize,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactReport {
    pub target: String,
    pub stale_index_warning: Option<String>,
    pub nodes: Vec<ImpactNode>,
    pub edges: Vec<ImpactEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactNode {
    pub id: String,
    pub label: String,
    pub file: String,
    pub kind: String,
    pub distance: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeReport {
    pub stale_index_warning: Option<String>,
    pub changed_files: Vec<String>,
    pub changed_symbols: Vec<ChangedSymbol>,
    pub affected: Vec<ImpactEdge>,
    pub risk_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangedSymbol {
    pub name: String,
    pub file: String,
    pub module: String,
    pub kind: String,
    pub line: usize,
    pub change_lines: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryResult {
    pub file: String,
    pub module: String,
    pub symbol: Option<String>,
    pub line: usize,
    pub score: f64,
    pub match_reasons: Vec<String>,
    pub resource: String,
    pub process: Option<String>,
}

pub fn symbol_context(repo_path: &str, symbol: &str) -> Result<SymbolContext> {
    let graph = RepoGraph::parse(repo_path)?;
    let Some((module, def)) = graph.find_symbol(symbol) else {
        return Err(AppError::Validation(format!("symbol not found: {symbol}")));
    };
    let defining_module = graph.resolver.module_name_for(module);
    let defining_file = path_string(&module.path);
    let callees = graph
        .calls_from(module, def)
        .into_iter()
        .map(|(called_module, called_def)| SymbolRelation {
            symbol: called_def.name.clone(),
            file: path_string(&called_module.path),
            module: graph.resolver.module_name_for(called_module),
            line: called_def.line,
            confidence: "medium".to_string(),
        })
        .collect();
    let callers = graph
        .callers_of(symbol)
        .into_iter()
        .filter(|(_, caller_def)| caller_def.name != def.name)
        .map(|(caller_module, caller_def)| SymbolRelation {
            symbol: caller_def.name.clone(),
            file: path_string(&caller_module.path),
            module: graph.resolver.module_name_for(caller_module),
            line: caller_def.line,
            confidence: "medium".to_string(),
        })
        .collect();

    Ok(SymbolContext {
        symbol: symbol.to_string(),
        defining_file,
        defining_module: defining_module.clone(),
        kind: format!("{:?}", def.kind).to_ascii_lowercase(),
        line: def.line,
        end_line: def.end_line,
        callers,
        callees,
        related_imports: module.imports.clone(),
        resources: vec![
            CONTEXT_URI.to_string(),
            format!("{MODULE_URI_PREFIX}{defining_module}"),
        ],
    })
}

pub fn impact(repo_path: &str, target: &str, max_depth: usize) -> Result<ImpactReport> {
    let graph = RepoGraph::parse(repo_path)?;
    let warning = stale_index_warning(Path::new(repo_path));
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([(target.to_string(), 0usize)]);

    while let Some((current, depth)) = queue.pop_front() {
        if depth > max_depth || !seen.insert(current.clone()) {
            continue;
        }
        if let Some((module, def)) = graph.find_symbol(&current) {
            nodes.push(ImpactNode {
                id: symbol_id(module, def),
                label: def.name.clone(),
                file: path_string(&module.path),
                kind: format!("{:?}", def.kind).to_ascii_lowercase(),
                distance: depth,
            });
            if depth < max_depth {
                for (caller_module, caller_def) in graph.callers_of(&current) {
                    edges.push(ImpactEdge {
                        source: symbol_id(caller_module, caller_def),
                        target: symbol_id(module, def),
                        kind: "CALLS".to_string(),
                        confidence: "medium".to_string(),
                    });
                    queue.push_back((caller_def.name.clone(), depth + 1));
                }
            }
        }

        for (source, import) in graph.importers_of(&current) {
            let source_module = graph.resolver.module_name_for(source);
            nodes.push(ImpactNode {
                id: format!("module:{source_module}"),
                label: source_module.clone(),
                file: path_string(&source.path),
                kind: "module".to_string(),
                distance: depth + 1,
            });
            edges.push(ImpactEdge {
                source: format!("module:{source_module}"),
                target: current.clone(),
                kind: "IMPORTS".to_string(),
                confidence: if import.name.as_deref() == Some(target) {
                    "high"
                } else {
                    "medium"
                }
                .to_string(),
            });
        }
    }

    nodes.sort_by(|a, b| a.distance.cmp(&b.distance).then_with(|| a.id.cmp(&b.id)));
    nodes.dedup_by(|a, b| a.id == b.id);
    edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    edges.dedup();

    Ok(ImpactReport {
        target: target.to_string(),
        stale_index_warning: warning,
        nodes,
        edges,
    })
}

pub fn detect_changes(repo_path: &str) -> Result<ChangeReport> {
    let root = Path::new(repo_path);
    validate_dir(root)?;
    let repo = git2::Repository::discover(root).map_err(|_| AppError::NotAGit(root.into()))?;
    let workdir = repo.workdir().unwrap_or(root);
    let diff = repo.diff_index_to_workdir(None, None)?;
    let changed_lines = RefCell::new(HashMap::<String, Vec<usize>>::new());
    diff.foreach(
        &mut |delta, _| {
            if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                changed_lines
                    .borrow_mut()
                    .entry(path_string(path))
                    .or_default();
            }
            true
        },
        None,
        None,
        Some(&mut |_delta, _hunk, line| {
            if line.origin() == '+'
                && let Some(path) = line
                    .new_lineno()
                    .and_then(|_| line.content().first().map(|_| ()))
                    .and_then(|_| _delta.new_file().path())
            {
                changed_lines
                    .borrow_mut()
                    .entry(path_string(path))
                    .or_default()
                    .push(line.new_lineno().unwrap_or(0) as usize);
            }
            true
        }),
    )?;
    let changed_lines = changed_lines.into_inner();

    let graph = RepoGraph::parse(workdir.to_str().unwrap_or(repo_path))?;
    let mut changed_symbols = Vec::new();
    for module in &graph.modules {
        let file = path_string(&module.path);
        let Some(lines) = changed_lines.get(&file) else {
            continue;
        };
        for def in &module.defs {
            let in_span = lines
                .iter()
                .copied()
                .filter(|line| *line >= def.line && *line <= def.end_line.max(def.line))
                .collect::<Vec<_>>();
            if !in_span.is_empty() {
                changed_symbols.push(ChangedSymbol {
                    name: def.name.clone(),
                    file: file.clone(),
                    module: graph.resolver.module_name_for(module),
                    kind: format!("{:?}", def.kind).to_ascii_lowercase(),
                    line: def.line,
                    change_lines: in_span,
                });
            }
        }
    }

    let mut affected = Vec::new();
    for symbol in &changed_symbols {
        affected.extend(impact(workdir.to_str().unwrap_or(repo_path), &symbol.name, 1)?.edges);
    }
    affected.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.target.cmp(&b.target))
    });
    affected.dedup();
    let risk_level = if changed_symbols.len() > 3 || affected.len() > 5 {
        "high"
    } else if !affected.is_empty() || changed_symbols.len() > 1 {
        "medium"
    } else {
        "low"
    }
    .to_string();

    Ok(ChangeReport {
        stale_index_warning: stale_index_warning(workdir),
        changed_files: changed_lines.into_keys().collect(),
        changed_symbols,
        affected,
        risk_level,
    })
}

pub fn query(repo_path: &str, query: &str, limit: usize) -> Result<Vec<QueryResult>> {
    let graph = RepoGraph::parse(repo_path)?;
    let processes = derive_processes(&graph.modules, &graph.resolver);
    let embedding_config =
        embeddings::config_for_repo(Path::new(repo_path), EmbeddingOptions::default())?;
    let (semantic_scores, _semantic_warnings) =
        embeddings::semantic_scores(Path::new(repo_path), query, &embedding_config)?;
    let terms = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let mut results = Vec::new();
    for module in &graph.modules {
        let module_name = graph.resolver.module_name_for(module);
        let module_text = module_name.to_ascii_lowercase();
        let file_text = path_string(&module.path).to_ascii_lowercase();
        let centrality = graph.importer_count(&module_name) as f64 * 0.25;
        let semantic = semantic_scores.get(&path_string(&module.path)).copied();
        let mut module_score = 0.0;
        let mut module_reasons = Vec::new();
        for term in &terms {
            if module_text.contains(term) || file_text.contains(term) {
                module_score += 1.0;
                module_reasons.push(format!("module:{term}"));
            }
        }
        if let Some(score) = semantic.filter(|score| *score > 0.0) {
            module_score += score * 1.5;
            module_reasons.push("semantic".to_string());
        }
        if module_score > 0.0 {
            results.push(QueryResult {
                file: path_string(&module.path),
                module: module_name.clone(),
                symbol: None,
                line: 1,
                score: module_score + centrality,
                match_reasons: module_reasons,
                resource: format!("{MODULE_URI_PREFIX}{module_name}"),
                process: None,
            });
        }

        for def in &module.defs {
            let name = def.name.to_ascii_lowercase();
            let mut score = 0.0;
            let mut reasons = Vec::new();
            for term in &terms {
                if name.contains(term) {
                    score += 2.0;
                    reasons.push(format!("symbol:{term}"));
                }
                if module
                    .source_lines
                    .get(def.line.saturating_sub(1))
                    .is_some_and(|line| line.to_ascii_lowercase().contains(term))
                {
                    score += 0.5;
                    reasons.push(format!("line:{term}"));
                }
            }
            if let Some(semantic_score) = semantic.filter(|score| *score > 0.0) {
                score += semantic_score * 0.5;
                reasons.push("semantic".to_string());
            }
            if score > 0.0 {
                results.push(QueryResult {
                    file: path_string(&module.path),
                    module: module_name.clone(),
                    symbol: Some(def.name.clone()),
                    line: def.line,
                    score: score + centrality,
                    match_reasons: reasons,
                    resource: format!("{MODULE_URI_PREFIX}{module_name}"),
                    process: process_for_symbol(&processes, &def.name),
                });
            }
        }
    }
    results.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    results.truncate(limit.max(1));
    Ok(results)
}

pub fn set_current_repo(repo_path: impl Into<PathBuf>) -> Result<()> {
    let path = repo_path.into();
    let root = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = CURRENT_REPO
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|err| AppError::Context(err.to_string()))?;
    *current = Some(root);
    Ok(())
}

pub fn resources() -> Result<Vec<Resource>> {
    let root = current_repo_root()?;
    let modules_count = indexed_modules(&root).map_or(0, |modules| modules.len());
    Ok(vec![
        resource(
            CONTEXT_URI,
            "RustRank context",
            "Agent-oriented repository summary",
        ),
        resource(SCHEMA_URI, "RustRank schema", "Graph manifest schema notes"),
        resource(MODULES_URI, "RustRank modules", "Indexed module list"),
        Annotated::new(
            RawResource::new(PROCESSES_URI, "RustRank processes")
                .with_description("Lightweight process flows inferred from graph edges")
                .with_mime_type("text/markdown")
                .with_size(modules_count as u32),
            None,
        ),
    ])
}

pub fn resource_templates() -> Vec<ResourceTemplate> {
    vec![
        template(
            "rustrank://repo/current/module/{name}",
            "RustRank module",
            "Context for one indexed module by module name",
        ),
        template(
            "rustrank://repo/current/process/{name}",
            "RustRank process",
            "Lightweight process flow by name",
        ),
    ]
}

pub fn read_current_resource(uri: &str) -> Result<String> {
    let root = current_repo_root()?;
    match uri {
        CONTEXT_URI => repo_context_resource(&root),
        SCHEMA_URI => schema_resource(&root),
        MODULES_URI => modules_resource(&root),
        PROCESSES_URI => processes_resource(&root),
        value if value.starts_with(MODULE_URI_PREFIX) => {
            module_resource(&root, &value[MODULE_URI_PREFIX.len()..])
        }
        value if value.starts_with(PROCESS_URI_PREFIX) => {
            process_resource(&root, &value[PROCESS_URI_PREFIX.len()..])
        }
        _ => Err(AppError::Validation(format!(
            "unknown RustRank resource URI: {uri}"
        ))),
    }
}

fn current_repo_root() -> Result<PathBuf> {
    if let Some(root) = CURRENT_REPO
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|err| AppError::Context(err.to_string()))?
        .clone()
    {
        return Ok(root);
    }
    Ok(std::env::current_dir()?)
}

fn repo_context_resource(root: &Path) -> Result<String> {
    let manifest = read_manifest(root)?;
    Ok(format!(
        "# RustRank repository context\n\nManifest: `.rustrank/index/v1/project_manifest.json`\n\nIndexed modules: {}\nGraph nodes: {}\nGraph edges: {}\n\nUse `context`, `impact`, `detect_changes`, and `query` for agent-facing analysis.\n",
        manifest["modules"].as_array().map_or(0, Vec::len),
        manifest["nodes"].as_array().map_or(0, Vec::len),
        manifest["graph_edges"].as_array().map_or(0, Vec::len),
    ))
}

fn schema_resource(root: &Path) -> Result<String> {
    let manifest = read_manifest(root)?;
    Ok(format!(
        "# RustRank graph schema\n\nNode kinds: file, module, symbol.\nEdge kinds: DEFINES, IMPORTS, UNRESOLVED_IMPORT, CALLS.\nFreshness: `{}`.\n",
        serde_json::to_string_pretty(&manifest["freshness"])?
    ))
}

fn modules_resource(root: &Path) -> Result<String> {
    let modules = indexed_modules(root)?;
    let mut out = String::from("# RustRank modules\n\n");
    for module in modules {
        out.push_str(&format!(
            "- `{}` `{}` symbols={} imports={}\n",
            module["module_name"].as_str().unwrap_or_default(),
            module["path"].as_str().unwrap_or_default(),
            module["symbol_count"].as_u64().unwrap_or_default(),
            module["import_count"].as_u64().unwrap_or_default(),
        ));
    }
    Ok(out)
}

fn module_resource(root: &Path, name: &str) -> Result<String> {
    let graph = RepoGraph::parse(root.to_str().unwrap_or("."))?;
    let Some(module) = graph
        .modules
        .iter()
        .find(|module| graph.resolver.module_name_for(module) == name)
    else {
        return Err(AppError::Validation(format!("module not found: {name}")));
    };
    let mut out = format!(
        "# Module `{name}`\n\nPath: `{}`\n\n",
        path_string(&module.path)
    );
    out.push_str("## Symbols\n\n");
    for def in &module.defs {
        out.push_str(&format!(
            "- `{}` {:?} lines {}-{}\n",
            def.name, def.kind, def.line, def.end_line
        ));
    }
    out.push_str("\n## Imports\n\n");
    for import in &module.imports {
        out.push_str(&format!(
            "- `{}` name={:?} line={}\n",
            import.module, import.name, import.line
        ));
    }
    Ok(out)
}

fn processes_resource(root: &Path) -> Result<String> {
    let graph = RepoGraph::parse(root.to_str().unwrap_or("."))?;
    let processes = derive_processes(&graph.modules, &graph.resolver);
    let mut out = String::from("# RustRank processes\n\n");
    for process in processes {
        out.push_str(&format!(
            "- `{}` from `{}` chain_steps={} resource=`{}{}`\n",
            process.name,
            process.module,
            process.call_chain.len(),
            PROCESS_URI_PREFIX,
            process.name
        ));
    }
    Ok(out)
}

fn process_resource(root: &Path, name: &str) -> Result<String> {
    let graph = RepoGraph::parse(root.to_str().unwrap_or("."))?;
    let processes = derive_processes(&graph.modules, &graph.resolver);
    let Some(process) = processes
        .iter()
        .find(|process| process.name == name || process.entry_symbol == name)
    else {
        return Err(AppError::Validation(format!("process not found: {name}")));
    };

    let mut out = format!(
        "# Process `{}`\n\nEntry: `{}` in `{}` line {}\n\n## Call chain\n\n",
        process.name, process.entry_symbol, process.file, process.line
    );
    for step in &process.call_chain {
        let indent = "  ".repeat(step.depth);
        let via = step
            .via
            .as_ref()
            .map(|via| format!(" via `{via}`"))
            .unwrap_or_default();
        out.push_str(&format!(
            "{indent}- `{}` `{}` line {}{}\n",
            step.symbol, step.module, step.line, via
        ));
    }
    Ok(out)
}

fn read_manifest(root: &Path) -> Result<serde_json::Value> {
    let path = root.join(".rustrank/index/v1/project_manifest.json");
    let text = std::fs::read_to_string(&path).map_err(|err| {
        AppError::Validation(format!(
            "RustRank manifest not found or unreadable at {}: {err}",
            path.display()
        ))
    })?;
    Ok(serde_json::from_str(&text)?)
}

fn indexed_modules(root: &Path) -> Result<Vec<serde_json::Value>> {
    Ok(read_manifest(root)?["modules"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

fn resource(uri: &str, name: &str, description: &str) -> Resource {
    Annotated::new(
        RawResource::new(uri, name)
            .with_description(description)
            .with_mime_type("text/markdown"),
        None,
    )
}

fn template(uri: &str, name: &str, description: &str) -> ResourceTemplate {
    Annotated::new(
        RawResourceTemplate::new(uri, name)
            .with_description(description)
            .with_mime_type("text/markdown"),
        None,
    )
}

struct RepoGraph {
    modules: Vec<ModuleDef>,
    resolver: LocalModuleResolver,
}

impl RepoGraph {
    fn parse(repo_path: &str) -> Result<Self> {
        let root = Path::new(repo_path);
        validate_dir(root)?;
        let ctx = Context::new(root.to_path_buf());
        let modules = ctx.parse_all()?;
        let resolver = LocalModuleResolver::new(&modules);
        Ok(Self { modules, resolver })
    }

    fn find_symbol(&self, symbol: &str) -> Option<(&ModuleDef, &Definition)> {
        self.modules.iter().find_map(|module| {
            module
                .defs
                .iter()
                .find(|def| def.name == symbol)
                .map(|def| (module, def))
        })
    }

    fn callers_of(&self, symbol: &str) -> Vec<(&ModuleDef, &Definition)> {
        self.modules
            .iter()
            .flat_map(|module| {
                module.defs.iter().filter_map(move |def| {
                    self.def_source(module, def)
                        .contains(&format!("{symbol}("))
                        .then_some((module, def))
                })
            })
            .collect()
    }

    fn calls_from(&self, module: &ModuleDef, def: &Definition) -> Vec<(&ModuleDef, &Definition)> {
        let source = self.def_source(module, def);
        let mut calls = Vec::new();
        for candidate_module in &self.modules {
            for candidate in &candidate_module.defs {
                if candidate.name != def.name && source.contains(&format!("{}(", candidate.name)) {
                    calls.push((candidate_module, candidate));
                }
            }
        }
        calls
    }

    fn importers_of(&self, target: &str) -> Vec<(&ModuleDef, &Import)> {
        self.modules
            .iter()
            .flat_map(|module| {
                module.imports.iter().filter_map(move |import| {
                    self.resolver
                        .import_matches_target(module, import, target)
                        .then_some((module, import))
                })
            })
            .collect()
    }

    fn importer_count(&self, module_name: &str) -> usize {
        self.modules
            .iter()
            .flat_map(|module| {
                module.imports.iter().filter(|import| {
                    self.resolver
                        .import_matches_target(module, import, module_name)
                })
            })
            .count()
    }

    fn def_source(&self, module: &ModuleDef, def: &Definition) -> String {
        let start = def.line.saturating_sub(1);
        let end = def.end_line.max(def.line).min(module.source_lines.len());
        module.source_lines[start..end].join("\n")
    }
}

fn stale_index_warning(root: &Path) -> Option<String> {
    let manifest = read_manifest(root).ok()?;
    let indexed = manifest["freshness"]["indexed_head"].as_str()?;
    let current = manifest["freshness"]["current_head"].as_str()?;
    (indexed != current)
        .then(|| format!("index was built at commit {indexed}, but current HEAD is {current}"))
}

fn symbol_id(module: &ModuleDef, def: &Definition) -> String {
    format!(
        "symbol:{}:{}:{}",
        module.language.config_name(),
        module.path.to_string_lossy().replace('\\', "/"),
        def.name
    )
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
