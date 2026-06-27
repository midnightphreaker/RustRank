use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    context::{
        Context, Definition, Import, Language, LocalModuleResolver, ModuleDef,
        all_supported_source_files, module_name_from_path, rel_string,
    },
    embeddings::{self, EmbeddingOptions, EmbeddingSource},
    error::{AppError, Result},
    process::{ProcessFlow, call_edges, derive_processes},
    project_config,
};

const SCHEMA_VERSION: u32 = 1;
const CACHE_VERSION: u32 = 1;
const EXTRACTOR_VERSION: &str = "tree-sitter-rustpython-v1";
const AGENTS_SECTION_START: &str = "<!-- rustrank-index:start -->";
const AGENTS_SECTION_END: &str = "<!-- rustrank-index:end -->";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexProjectResponse {
    pub cache_dir: String,
    pub project_manifest: String,
    pub cache_version: u32,
    pub scanned_files: usize,
    pub indexed_files: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub stale_removed: usize,
    pub languages: Vec<LanguageIndexSummary>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanguageIndexSummary {
    pub language: Language,
    pub files: usize,
    pub symbols: usize,
    pub imports: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub index_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CacheHeader {
    schema_version: u32,
    cache_version: u32,
    rustrank_version: String,
    extractor_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FileHash {
    algorithm: String,
    value: String,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CachedFileFacts {
    header: CacheHeader,
    path: String,
    module_name: String,
    language: Language,
    content_hash: FileHash,
    symbols: Vec<Definition>,
    imports: Vec<Import>,
    declared_namespaces: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LanguageIndex {
    header: CacheHeader,
    language: Language,
    files: Vec<LanguageFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LanguageFileEntry {
    path: String,
    module_name: String,
    language: Language,
    content_hash: FileHash,
    cache_file: String,
    symbol_count: usize,
    import_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProjectManifest {
    header: CacheHeader,
    languages: Vec<LanguageShardRef>,
    modules: Vec<ProjectModule>,
    edges: Vec<ProjectImportEdge>,
    unresolved_imports: Vec<ProjectImportEdge>,
    nodes: Vec<GraphNode>,
    graph_edges: Vec<GraphEdge>,
    processes: Vec<ProcessFlow>,
    freshness: Freshness,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LanguageShardRef {
    language: Language,
    index_file: String,
    file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProjectModule {
    id: String,
    language: Language,
    path: String,
    module_name: String,
    symbol_count: usize,
    import_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProjectImportEdge {
    source_id: String,
    source_module: String,
    source_path: String,
    target_module: String,
    target_id: Option<String>,
    target_path: Option<String>,
    import_module: String,
    import_name: Option<String>,
    line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct GraphNode {
    schema: String,
    id: String,
    kind: String,
    name: String,
    path: Option<String>,
    language: Option<Language>,
    line: Option<usize>,
    end_line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct GraphEdge {
    schema: String,
    kind: String,
    source: String,
    target: String,
    line: Option<usize>,
    confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Freshness {
    indexed_head: String,
    current_head: String,
    stale: bool,
}

pub fn index_project(
    repo_path: &str,
    languages: Option<Vec<String>>,
    force_rebuild: bool,
    clean_stale: bool,
) -> Result<IndexProjectResponse> {
    index_project_with_embeddings(
        repo_path,
        languages,
        force_rebuild,
        clean_stale,
        EmbeddingOptions::default(),
    )
}

pub fn index_project_with_embeddings(
    repo_path: &str,
    languages: Option<Vec<String>>,
    force_rebuild: bool,
    clean_stale: bool,
    embedding_options: EmbeddingOptions,
) -> Result<IndexProjectResponse> {
    let root = Path::new(repo_path);
    if !root.is_dir() {
        return Err(AppError::Validation(format!(
            "repository path is not a directory: {}",
            root.display()
        )));
    }

    let (enabled_languages, mut warnings) = languages_from_request(root, languages)?;
    let cache_root = root.join(".rustrank/index/v1");
    let languages_root = cache_root.join("languages");
    std::fs::create_dir_all(&languages_root)?;

    let ctx = Context::new(root.to_path_buf());
    let mut modules = Vec::new();
    let mut summaries = Vec::new();
    let mut total_hits = 0;
    let mut total_misses = 0;
    let mut referenced_cache_files = HashSet::new();
    let mut embedding_sources = Vec::new();
    let mut indexed_files = Vec::new();

    let files = all_supported_source_files(root)?
        .into_iter()
        .filter(|(_, language)| enabled_languages.contains(language))
        .collect::<Vec<_>>();

    for language in Language::ALL {
        if !enabled_languages.contains(&language) {
            continue;
        }

        let language_dir = languages_root.join(language.config_name());
        let files_dir = language_dir.join("files");
        std::fs::create_dir_all(&files_dir)?;

        let mut facts = Vec::new();
        let mut hits = 0;
        let mut misses = 0;

        for (path, _) in files
            .iter()
            .filter(|(_, file_language)| *file_language == language)
        {
            let rel_path = rel_string(root, path)?;
            let file_hash = hash_file(path)?;
            let content = match std::fs::read_to_string(path) {
                Ok(content) => content,
                Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                    warnings.push(format!("skipped non-UTF-8 source file `{rel_path}`: {err}"));
                    continue;
                }
                Err(err) => return Err(err.into()),
            };
            let cache_file_name = format!("{}.json", file_hash.value);
            let cache_file = files_dir.join(&cache_file_name);
            let maybe_cached = (!force_rebuild)
                .then(|| read_cached_file(&cache_file, &file_hash, &rel_path, &mut warnings))
                .transpose()?
                .flatten();

            let fact = if let Some(fact) = maybe_cached {
                hits += 1;
                fact
            } else {
                misses += 1;
                let module = ctx.get_or_parse(rel_path.clone())?;
                cached_file_facts(&module, file_hash.clone())
            };

            referenced_cache_files.insert(cache_file);
            embedding_sources.push(EmbeddingSource {
                path: rel_path,
                content_hash: file_hash.value.clone(),
                content,
            });
            write_json_atomic(&files_dir.join(cache_file_name), &fact)?;
            modules.push(module_from_fact(&fact));
            indexed_files.push((path.clone(), language));
            facts.push(fact);
        }

        facts.sort_by(|a, b| a.path.cmp(&b.path));
        let language_index = language_index(language, &facts);
        let index_file = language_dir.join("index.json");
        write_json_atomic(&index_file, &language_index)?;

        summaries.push(LanguageIndexSummary {
            language,
            files: facts.len(),
            symbols: facts.iter().map(|fact| fact.symbols.len()).sum(),
            imports: facts.iter().map(|fact| fact.imports.len()).sum(),
            cache_hits: hits,
            cache_misses: misses,
            index_file: index_file
                .strip_prefix(root)
                .unwrap_or(&index_file)
                .to_string_lossy()
                .replace('\\', "/"),
        });

        total_hits += hits;
        total_misses += misses;
    }

    let stale_removed = if clean_stale {
        clean_stale_cache_files(&languages_root, &referenced_cache_files)?
    } else {
        0
    };

    let embedding_config = embeddings::config_for_repo(root, embedding_options)?;
    let embedding_stats =
        embeddings::index_embeddings(root, &embedding_config, &embedding_sources)?;
    warnings.extend(embedding_stats.warnings);

    let source_modules = source_modules(root, &ctx, &indexed_files)?;
    let manifest = project_manifest(root, &summaries, &source_modules);
    let manifest_path = cache_root.join("project_manifest.json");
    write_json_atomic(&manifest_path, &manifest)?;
    update_agents_md(root, &summaries, &cache_root, &manifest_path, &mut warnings)?;
    write_workflow_docs(root)?;

    warnings.extend(
        project_config::configured_languages(root)?
            .invalid
            .into_iter()
            .map(|name| format!("ignored unsupported configured language {name:?}")),
    );
    warnings.sort();
    warnings.dedup();

    Ok(IndexProjectResponse {
        cache_dir: cache_root.to_string_lossy().replace('\\', "/"),
        project_manifest: manifest_path.to_string_lossy().replace('\\', "/"),
        cache_version: CACHE_VERSION,
        scanned_files: files.len(),
        indexed_files: modules.len(),
        cache_hits: total_hits,
        cache_misses: total_misses,
        stale_removed,
        languages: summaries,
        warnings,
    })
}

fn languages_from_request(
    root: &Path,
    languages: Option<Vec<String>>,
) -> Result<(Vec<Language>, Vec<String>)> {
    let Some(raw_languages) = languages else {
        return Ok((project_config::enabled_languages(root)?, Vec::new()));
    };

    let mut enabled = Vec::new();
    let mut warnings = Vec::new();
    for raw in raw_languages {
        match Language::from_config_name(&raw) {
            Some(language) if !enabled.contains(&language) => enabled.push(language),
            Some(_) => {}
            None => warnings.push(format!("ignored unsupported requested language {raw:?}")),
        }
    }

    if enabled.is_empty() {
        return Err(AppError::Validation(
            "no supported languages requested".to_string(),
        ));
    }

    enabled.sort_by_key(|language| language.order());
    Ok((enabled, warnings))
}

fn cached_file_facts(module: &ModuleDef, content_hash: FileHash) -> CachedFileFacts {
    CachedFileFacts {
        header: cache_header(),
        path: module.path.to_string_lossy().replace('\\', "/"),
        module_name: module_name_from_path(&module.path),
        language: module.language,
        content_hash,
        symbols: module.defs.clone(),
        imports: module.imports.clone(),
        declared_namespaces: module.declared_namespaces.clone(),
    }
}

fn module_from_fact(fact: &CachedFileFacts) -> ModuleDef {
    ModuleDef {
        path: PathBuf::from(&fact.path),
        language: fact.language,
        imports: fact.imports.clone(),
        defs: fact.symbols.clone(),
        source_lines: Vec::new(),
        declared_namespaces: fact.declared_namespaces.clone(),
    }
}

fn language_index(language: Language, facts: &[CachedFileFacts]) -> LanguageIndex {
    LanguageIndex {
        header: cache_header(),
        language,
        files: facts
            .iter()
            .map(|fact| LanguageFileEntry {
                path: fact.path.clone(),
                module_name: fact.module_name.clone(),
                language: fact.language,
                content_hash: fact.content_hash.clone(),
                cache_file: format!(
                    ".rustrank/index/v1/languages/{}/files/{}.json",
                    language.config_name(),
                    fact.content_hash.value
                ),
                symbol_count: fact.symbols.len(),
                import_count: fact.imports.len(),
            })
            .collect(),
    }
}

fn project_manifest(
    root: &Path,
    summaries: &[LanguageIndexSummary],
    modules: &[ModuleDef],
) -> ProjectManifest {
    let resolver = LocalModuleResolver::new(modules);
    let module_by_name = modules
        .iter()
        .map(|module| (resolver.module_name_for(module), module))
        .collect::<HashMap<_, _>>();

    let mut edges = Vec::new();
    let mut unresolved_imports = Vec::new();
    for module in modules {
        let source_module = resolver.module_name_for(module);
        let source_id = module_id(module.language, &module.path);
        let source_path = module.path.to_string_lossy().replace('\\', "/");
        for import in &module.imports {
            let targets = resolver.resolve_import_targets(module, import);
            if targets.is_empty() {
                unresolved_imports.push(ProjectImportEdge {
                    source_id: source_id.clone(),
                    source_module: source_module.clone(),
                    source_path: source_path.clone(),
                    target_module: import.module.clone(),
                    target_id: None,
                    target_path: None,
                    import_module: import.module.clone(),
                    import_name: import.name.clone(),
                    line: import.line,
                });
                continue;
            }
            for target in targets {
                let target_module = module_by_name.get(&target);
                edges.push(ProjectImportEdge {
                    source_id: source_id.clone(),
                    source_module: source_module.clone(),
                    source_path: source_path.clone(),
                    target_module: target.clone(),
                    target_id: target_module.map(|module| module_id(module.language, &module.path)),
                    target_path: target_module
                        .map(|module| module.path.to_string_lossy().replace('\\', "/")),
                    import_module: import.module.clone(),
                    import_name: import.name.clone(),
                    line: import.line,
                });
            }
        }
    }

    edges.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then_with(|| a.target_module.cmp(&b.target_module))
            .then_with(|| a.line.cmp(&b.line))
    });
    unresolved_imports.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then_with(|| a.import_module.cmp(&b.import_module))
            .then_with(|| a.line.cmp(&b.line))
    });

    let mut project_modules = modules
        .iter()
        .map(|module| ProjectModule {
            id: module_id(module.language, &module.path),
            language: module.language,
            path: module.path.to_string_lossy().replace('\\', "/"),
            module_name: resolver.module_name_for(module),
            symbol_count: module.defs.len(),
            import_count: module.imports.len(),
        })
        .collect::<Vec<_>>();
    project_modules.sort_by(|a, b| a.id.cmp(&b.id));
    let (nodes, graph_edges) = graph_shape(modules, &resolver, &edges, &unresolved_imports);
    let processes = derive_processes(modules, &resolver);
    let freshness = freshness(root);

    ProjectManifest {
        header: cache_header(),
        languages: summaries
            .iter()
            .map(|summary| LanguageShardRef {
                language: summary.language,
                index_file: summary.index_file.clone(),
                file_count: summary.files,
            })
            .collect(),
        modules: project_modules,
        edges,
        unresolved_imports,
        nodes,
        graph_edges,
        processes,
        freshness,
    }
}

fn graph_shape(
    modules: &[ModuleDef],
    resolver: &LocalModuleResolver,
    imports: &[ProjectImportEdge],
    unresolved: &[ProjectImportEdge],
) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for module in modules {
        let file_id = file_node_id(module);
        let module_name = resolver.module_name_for(module);
        let module_id = format!("module:{module_name}");
        let path = module.path.to_string_lossy().replace('\\', "/");
        nodes.push(GraphNode {
            schema: "graph_node".to_string(),
            id: file_id.clone(),
            kind: "file".to_string(),
            name: path.clone(),
            path: Some(path.clone()),
            language: Some(module.language),
            line: None,
            end_line: None,
        });
        nodes.push(GraphNode {
            schema: "graph_node".to_string(),
            id: module_id.clone(),
            kind: "module".to_string(),
            name: module_name,
            path: Some(path),
            language: Some(module.language),
            line: None,
            end_line: None,
        });
        edges.push(GraphEdge {
            schema: "graph_edge".to_string(),
            kind: "DEFINES".to_string(),
            source: file_id,
            target: module_id.clone(),
            line: None,
            confidence: "high".to_string(),
        });

        for def in &module.defs {
            let symbol_id = symbol_node_id(module, def);
            nodes.push(GraphNode {
                schema: "graph_node".to_string(),
                id: symbol_id.clone(),
                kind: "symbol".to_string(),
                name: def.name.clone(),
                path: Some(module.path.to_string_lossy().replace('\\', "/")),
                language: Some(module.language),
                line: Some(def.line),
                end_line: Some(def.end_line),
            });
            edges.push(GraphEdge {
                schema: "graph_edge".to_string(),
                kind: "DEFINES".to_string(),
                source: module_id.clone(),
                target: symbol_id,
                line: Some(def.line),
                confidence: "high".to_string(),
            });
        }
    }

    for edge in call_edges(modules, resolver) {
        edges.push(GraphEdge {
            schema: "graph_edge".to_string(),
            kind: "CALLS".to_string(),
            source: format!(
                "symbol:{}:{}:{}",
                Language::from_path(Path::new(&edge.source_file))
                    .map(|language| language.config_name())
                    .unwrap_or("unknown"),
                edge.source_file,
                edge.source_symbol
            ),
            target: format!(
                "symbol:{}:{}:{}",
                crate::context::Language::from_path(Path::new(&edge.target_file))
                    .map(|language| language.config_name())
                    .unwrap_or("unknown"),
                edge.target_file,
                edge.target_symbol
            ),
            line: Some(edge.call_line),
            confidence: "medium".to_string(),
        });
    }

    for edge in imports {
        if edge.target_id.is_some() {
            edges.push(GraphEdge {
                schema: "graph_edge".to_string(),
                kind: "IMPORTS".to_string(),
                source: format!("module:{}", edge.source_module),
                target: format!("module:{}", edge.target_module),
                line: Some(edge.line),
                confidence: "high".to_string(),
            });
        }
    }
    for edge in unresolved {
        edges.push(GraphEdge {
            schema: "graph_edge".to_string(),
            kind: "UNRESOLVED_IMPORT".to_string(),
            source: format!("module:{}", edge.source_module),
            target: edge.target_module.clone(),
            line: Some(edge.line),
            confidence: "low".to_string(),
        });
    }

    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    nodes.dedup_by(|a, b| a.id == b.id);
    edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    edges.dedup();
    (nodes, edges)
}

fn file_node_id(module: &ModuleDef) -> String {
    format!("file:{}", module.path.to_string_lossy().replace('\\', "/"))
}

fn symbol_node_id(module: &ModuleDef, def: &Definition) -> String {
    format!(
        "symbol:{}:{}:{}",
        module.language.config_name(),
        module.path.to_string_lossy().replace('\\', "/"),
        def.name
    )
}

fn freshness(root: &Path) -> Freshness {
    let (indexed_head, current_head) = git_head(root).unwrap_or_else(|| {
        let value = "unknown".to_string();
        (value.clone(), value)
    });
    Freshness {
        stale: indexed_head != current_head,
        indexed_head,
        current_head,
    }
}

fn git_head(root: &Path) -> Option<(String, String)> {
    let repo = git2::Repository::discover(root).ok()?;
    let head = repo.head().ok()?.target()?.to_string();
    Some((head.clone(), head))
}

fn module_id(language: Language, path: &Path) -> String {
    format!(
        "{}:{}",
        language.config_name(),
        path.to_string_lossy().replace('\\', "/")
    )
}

fn read_cached_file(
    cache_file: &Path,
    expected_hash: &FileHash,
    expected_path: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<CachedFileFacts>> {
    if !cache_file.exists() {
        return Ok(None);
    }
    let source = match std::fs::read_to_string(cache_file) {
        Ok(source) => source,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            warnings.push(format!(
                "ignored unreadable cache file `{}`: {}",
                cache_file.display(),
                err
            ));
            return Ok(None);
        }
    };
    let fact = match serde_json::from_str::<CachedFileFacts>(&source) {
        Ok(fact) => fact,
        Err(err) => {
            warnings.push(format!(
                "ignored corrupt cache file `{}`: {}",
                cache_file.display(),
                err
            ));
            return Ok(None);
        }
    };
    if fact.header == cache_header()
        && fact.content_hash == *expected_hash
        && fact.path == expected_path
    {
        Ok(Some(fact))
    } else {
        Ok(None)
    }
}

fn hash_file(path: &Path) -> Result<FileHash> {
    let bytes = std::fs::read(path)?;
    Ok(FileHash {
        algorithm: "blake3".to_string(),
        value: blake3::hash(&bytes).to_hex().to_string(),
        size_bytes: bytes.len() as u64,
    })
}

fn cache_header() -> CacheHeader {
    CacheHeader {
        schema_version: SCHEMA_VERSION,
        cache_version: CACHE_VERSION,
        rustrank_version: env!("CARGO_PKG_VERSION").to_string(),
        extractor_version: EXTRACTOR_VERSION.to_string(),
    }
}

fn source_modules(
    root: &Path,
    ctx: &Context,
    files: &[(PathBuf, Language)],
) -> Result<Vec<ModuleDef>> {
    let mut modules = Vec::new();
    for (path, _) in files {
        modules.push(ctx.get_or_parse(rel_string(root, path)?)?);
    }
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(modules)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn update_agents_md(
    root: &Path,
    summaries: &[LanguageIndexSummary],
    cache_root: &Path,
    manifest_path: &Path,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let agents_path = root.join("AGENTS.md");
    let existing = if agents_path.exists() {
        match std::fs::read_to_string(&agents_path) {
            Ok(existing) => existing,
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                warnings.push(format!(
                    "skipped AGENTS.md update because `{}` is not valid UTF-8: {}",
                    agents_path.display(),
                    err
                ));
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        }
    } else {
        "# AGENTS.md\n".to_string()
    };
    let section = agents_index_section(root, summaries, cache_root, manifest_path)?;
    let updated = replace_or_append_agents_section(&existing, &section);
    std::fs::write(agents_path, updated)?;
    Ok(())
}

fn agents_index_section(
    root: &Path,
    summaries: &[LanguageIndexSummary],
    cache_root: &Path,
    manifest_path: &Path,
) -> Result<String> {
    let cache_dir = repo_relative_path(root, cache_root)?;
    let manifest = repo_relative_path(root, manifest_path)?;
    let mut section = format!(
        "{AGENTS_SECTION_START}\n\
## RustRank Indexed Codebase\n\n\
RustRank indexed this repository with language-specific analyzers. Use the persistent cache and project manifest for repository-level symbol/import context before broad code changes.\n\n\
Persistent index cache: `{cache_dir}/`\n\
Project manifest: `{manifest}`\n\n\
MCP resources: `rustrank://repo/current/context`, `rustrank://repo/current/schema`, `rustrank://repo/current/modules`, `rustrank://repo/current/module/{{name}}`, `rustrank://repo/current/processes`, and `rustrank://repo/current/process/{{name}}`.\n\n\
Agent-facing tools: `context`, `impact`, `detect_changes`, and `query`. Use `context` before editing a symbol, `impact` before changing shared APIs, `detect_changes` before final review, and `query` for graph-aware repository search.\n\n\
Workflow docs: `.rustrank/skills/exploring.md`, `.rustrank/skills/impact-analysis.md`, `.rustrank/skills/debugging.md`, `.rustrank/skills/refactoring.md`.\n\n\
Language index shards:\n\n\
| Language | Files | Symbols | Imports |\n\
| --- | ---: | ---: | ---: |\n",
    );

    for summary in summaries {
        section.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            summary.language.config_name(),
            summary.files,
            summary.symbols,
            summary.imports
        ));
    }

    section.push_str(
        "\nThe cache stores per-file symbols, imports, declared namespaces, content hashes, graph nodes, graph edges, and git freshness metadata. It does not store source lines, snippets, or absolute paths. Re-run `index_project` after source changes to refresh this section and the persistent cache.\n",
    );
    section.push_str(AGENTS_SECTION_END);
    section.push('\n');
    Ok(section)
}

fn write_workflow_docs(root: &Path) -> Result<()> {
    let dir = root.join(".rustrank/skills");
    std::fs::create_dir_all(&dir)?;
    for (name, body) in [
        (
            "exploring.md",
            "# RustRank Exploring\n\n1. Run `index_project`.\n2. Read `rustrank://repo/current/context`.\n3. Use `query` to find entry points and central modules.\n",
        ),
        (
            "impact-analysis.md",
            "# RustRank Impact Analysis\n\n1. Use `context` for the target symbol.\n2. Use `impact` with an appropriate depth.\n3. Verify high-confidence edges against source before editing.\n",
        ),
        (
            "debugging.md",
            "# RustRank Debugging\n\n1. Search errors with existing trace tools.\n2. Use `query` for related code.\n3. Use `context` and `impact` to inspect likely symbols and callers.\n",
        ),
        (
            "refactoring.md",
            "# RustRank Refactoring\n\n1. Use `impact` before changing public symbols.\n2. Make the smallest source change.\n3. Run `detect_changes` and tests before finalizing.\n",
        ),
    ] {
        std::fs::write(dir.join(name), body)?;
    }
    Ok(())
}

fn repo_relative_path(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|err| AppError::Context(err.to_string()))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

fn replace_or_append_agents_section(existing: &str, section: &str) -> String {
    let mut normalized = existing.trim_end().to_string();
    let Some(start) = normalized.find(AGENTS_SECTION_START) else {
        if !normalized.is_empty() {
            normalized.push_str("\n\n");
        }
        normalized.push_str(section.trim_end());
        normalized.push('\n');
        return normalized;
    };
    let Some(end_offset) = normalized[start..].find(AGENTS_SECTION_END) else {
        normalized.push_str("\n\n");
        normalized.push_str(section.trim_end());
        normalized.push('\n');
        return normalized;
    };

    let end = start + end_offset + AGENTS_SECTION_END.len();
    normalized.replace_range(start..end, section.trim_end());
    normalized.push('\n');
    normalized
}

fn clean_stale_cache_files(root: &Path, referenced: &HashSet<PathBuf>) -> Result<usize> {
    if !root.exists() {
        return Ok(0);
    }

    let mut removed = 0;
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "index.json") {
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "json") && !referenced.contains(path) {
            std::fs::remove_file(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}
