# RustRank MCP Server Design

> Rust rewrite of search-tools-mcp (Python) — 12 MCP tools for repository analysis powered by Rust-native code understanding.

## Overview

| **Aspect** | **Decision** |
|---|---|
| Project | New crate: `rustank` (single binary) |
| Cargo Workspace | `/search-tools-mcp/Cargo.toml` (monorepo with existing Python project) |
| MCP Framework | `rmcp = "1.7.0"` with `#[tool]` / `#[tool_router]` procedural macros |
| Transport | Streamable HTTP (primary) + STDIO (fallback via `serve_server()`) |
| Python AST | `rustpython-parser = "0.4.0"` — parse-once, share-everywhere cache |
| Graph Search | `rustpython-parser` AST + `regex` for symbol name matching |
| GraphOps | `graph = "0.0.8"` for CodeRank (bidirectional PageRank on import graphs) |
| Git | `git2 = "0.20"` — optional, loaded only when repo_analysis / repo_symbols are called |
| Testing | Real repos on disk under `tests/fixtures/` — zero mocks |

## Project Structure

```
search-tools-mcp/
├── Cargo.toml                          # Workspace root — defines members
├── packages/
│   └── rustank/                        # RustRank MCP server
│       ├── Cargo.toml                  # name = "rustank", edition = "2024"
│       ├── src/
│       │   ├── main.rs                 # Entry point — initializes Router, Server, transport
│       │   ├── router.rs               # Router composition — all tool routers
│       │   ├── server.rs               # Server config — transports, auth, middleware
│       │   ├── context.rs              # Shared RepoContext + ParseCache
│       │   ├── error.rs                # AppError enum + Result<T>
│       │   ├── fmt.rs                  # Response formatting + table rendering
│       │   ├── analyzer/               # Code analysis — CodeRank, hotspots, trace
│       │   │   ├── mod.rs
│       │   │   ├── coderank.rs         # Bidirectional PageRank implementation
│       │   │   ├── hotspots.rs         # Code hotspots analysis (merge + frequency)
│       │   │   ├── data_flow.rs        # Data flow tracing across modules
│       │   │   └── trace.rs            # Dependency / call chain tracing
│       │   ├── tools/                  # MCP tool implementations
│       │   │   ├── mod.rs
│       │   │   ├── search.rs           # contextual_search, smart_code_search, api_usage
│       │   │   ├── code_rank.rs        # coderank_analysis, code_hotspots
│       │   │   ├── trace.rs            # trace_data_flow, feature_impl, dep_impact
│       │   │   ├── analysis.rs         # error_patterns, perf_bottleneck, exec_path
│       │   │   ├── git.rs              # repo_analysis, repo_symbols
│       │   │   └── config.rs           # get_config, set_config
│       │   └── parser/                 # Python AST parsing layer
│       │       ├── mod.rs
│       │       ├── symbols.rs          # Import/def extraction, name resolution
│       │       └── tokens.rs           # Token stream & pattern matching
│       ├── tests/
│       │   ├── mod.rs                  # Integration test entry point
│       │   ├── fixtures.rs             # Fixture management (copy real repos)
│       │   ├── snapshot/               # Expected output comparisons
│       │   │   ├── coderank.py         # Python-side CodeRank for verification
│       │   │   └── hotspots.py         # Python hotspot analysis reference
│       │   └── integration/            # End-to-end tool tests
│       │       ├── test_search.rs
│       │       ├── test_coderank.rs
│       │       ├── test_trace.rs
│       │       ├── test_analysis.rs
│       │       └── test_git.rs
│       └── benches/                    # Micro-benchmarks (optional)
├── .superpowers/brainstorm/            # brainstorm output (visual companion)
├── docs/
│   └── superpowers/
│       ├── specs/
│       │   └── 2026-05-16-rustank-design.md  ← this file
│       └── plans/
│           └── 2026-05-16-rustank-plan.md     # Implementation plan (future)
```

## Components

### 1. Cargo.toml

```toml
[package]
name = "rustank"
version = "0.1.0"
edition = "2024"
description = "RustNative MCP Server — 12 tools for repository analysis"

[dependencies]
# MCP framework
rmcp = { version = "1.7.0", features = ["tokio"] }
axum = "0.8"

# Python AST parsing
rustpython-parser = "0.4.0"
rustpython-ast = "0.4.0"
rustpython-compiler = "0.4.0"

# Graph & CodeRank
graph = "0.0.8"

# Git integration (optional)
git2 = { version = "0.20", optional = true }

# Utilities
regex = "1.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
thiserror = "2"
itertools = "0.14"
indexmap = "2"
walkdir = "2"
tempfile = "3"
chrono = "0.4"

# Conditional git feature
[features]
default = []
git = ["dep:git2"]
```

#### Dependency Rationale

| Crate | Purpose | Why this crate |
|---|---|---|
| `rmcp = "1.7.0"` | MCP protocol tools | Native Rust implementation with `#[tool]` macros |
| `axum = "0.8"` | HTTP server (Streamable HTTP) | rmcp's transport layer; tokio-native, async |
| `rustpython-parser = "0.4.0"` | Python AST parsing | Same engine as Ruff; produces complete AST |
| `rustpython-ast = "0.4.0"` | AST node definitions | `Expr`, `Stmt`, `Identifier`, `mod::Mod` types |
| `graph = "0.0.8"` | CodeRank (bidirectional PageRank) | Pure Rust; `Node::new`, `graph.add_node`, `pagerank()` |
| `git2 = "0.20"` | Git repo analysis | Bindings to libgit2; `Repository::open`, `Commit`, `Diff` |
| `regex = "1.10"` | Name matching, symbol search | RegexSet for multi-pattern queries |
| `serde` + `serde_json` | JSON tool input/output | `#[derive(Serialize, Deserialize)]` on all tool schemas |
| `thiserror = "2"` | Error types | `AppError` enum with contextual messages |
| `itertools = "0.14"` | Iterator utilities | `multi_cartesian_product`, flatten chains, grouping |
| `indexmap = "2"` | Ordered mappings | AST cache, symbol maps — preserves insertion order |
| `walkdir = "2"` | Recursive file scanning | Source file discovery across project trees |
| `tempfile / chrono` | Utilities | Fixture temp dirs, timestamp formatting |

### 2. Main Entry Point — `src/main.rs`

```rust
use rmcp::{Handler, Router, Server, service::RpcServiceExt};
use tokio::net::TcpListener;
use rustank::context::SharedContext;
use rustank::router::create_router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("rustank=info,rmcp=warn")
        .init();

    // Parse CLI args or environment for transport mode
    let context = SharedContext::new();
    let router = create_router(context);

    // Streamable HTTP mode (primary)
    if std::env::var("RUSTANK_TRANSPORT").ok() == Some("streamable_http".into()) {
        let addr = std::env::var("RUSTANK_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:63477".into());
        let listener = TcpListener::bind(addr).await?;
        tracing::info!("Listening on {addr}");
        let service = Router::new(router).into_service();
        axum::serve(listener, service).await?;
    } else {
        // STDIO mode (default)
        let service = Router::new(router).into_service();
        let server = Server::new().add_module(rustank::tools::ALL_TOOLS);
        server.serve_server(service).await?;
    }

    Ok(())
}
```

### 3. Context — `src/context.rs`

Shared state passed to all tools:

```rust
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashMap;
use indexmap::IndexMap;
use rustpython_parser::ast::Mod;

/// Core analysis context — shared by ALL tool handlers.
/// Immutable after creation except for the parse cache (RwLock).
pub struct RepoContext {
    /// Absolute path to the repository root.
    pub path: PathBuf,

    /// Root Python source directory for this repo.
    /// May differ from `path` if sources are in src/ or packages/.
    pub src_root: PathBuf,

    /// Cached ASTs — parse-once, share-everywhere.
    /// Key: relative path from src_root.
    /// Value: (AST, imports, defs, comments) tuple.
    parse_cache: Arc<std::sync::RwLock<IndexMap<String, CacheEntry>>>,

    /// Git repo handle (lazy-initialized).
    /// Only present if `git2` feature is enabled and repo is valid.
    git_repo: Option<git2::Repository>,
}

/// Single parsed Python file result.
pub struct CacheEntry {
    /// Full AST tree.
    pub ast: Mod,
    /// All imports and from-imports found in the module.
    pub imports: Vec<Import>,
    /// All function/class/variable definitions.
    pub defs: Vec<Def>,
    /// All top-level and docstring comments.
    pub comments: HashMap<usize, String>,
}

pub struct Import {
    pub name: String,        // "requests" or "os.path"
    pub alias: Option<String>, // "import requests as req" → Some("req")
    pub is_from: bool,          // "from x import y"
    pub imported_name: Option<String>, // For `from`, the name being imported
}

pub struct Def {
    pub name: String,
    pub kind: DefKind,
    pub line_no: usize,
    pub end_line: usize,
    pub docstring: Option<String>,
}

pub enum DefKind {
    Function {
        args: Vec<Arg>,
        is_async: bool,
        return_type: Option<String>,
    },
    Class {
        superclasses: Vec<String>,
    },
    Variable,
}

pub struct Arg {
    pub name: String,
    pub kind: ArgKind,
}

pub enum ArgKind {
    Positional,
    KeywordOnly,
    VariadicParams,   // /varargs (i.e., *args)
    VariadicKeywords,  // **kwargs
}

impl RepoContext {
    pub fn new(path: PathBuf, src_root: Option<PathBuf>) -> Self {
        let git_repo = std::fs::metadata(&path).map(|meta| {
            git2::Repository::open(path.join(".git")).ok()
        })
        .ok()
        .flatten();

        Self {
            path,
            src_root: src_root.unwrap_or_else(|| path.clone()),
            parse_cache: Arc::new(std::sync::RwLock::new(IndexMap::new())),
            git_repo,
        }
    }

    /// Parse a Python file, using cache if available (parse-once).
    pub fn parse_file(&self, rel_path: &str) -> Result<Arc<CacheEntry>> {
        let mut cache = self.parse_cache.write().unwrap();
        if let Some(entry) = cache.get(rel_path) {
            // Return Arc-wrapped clone for cheap sharing.
            return Ok(Arc::new(entry.clone()));
        }
        // Read file, parse with rustpython_parser, cache result.
        let full_path = self.src_root.join(rel_path);
        let src = std::fs::read_to_string(&full_path)
            .map_err(|e| anyhow::anyhow!("Failed to read {full_path:?}: {e}"))?;
        let ast = rustpython_parser::parse_module(&src, &full_path.to_string_lossy()).map_err(|e| {
            anyhow::anyhow!("Parse error in {rel_path}: {e}")
        })?;
        // Extract imports, defs, comments from AST.
        let imports = extract_imports(&ast);
        let defs = extract_defs(&ast);
        let comments = extract_comments(&src);
        let entry = CacheEntry { ast, imports, defs, comments };
        // NOTE: CacheEntry does NOT implement Clone. To make this work,
        // wrap in Arc. Alternatively, store Arc<CacheEntry> directly.
        cache.insert(rel_path.to_string(), Arc::new(entry));
        Ok(Arc::new(entry))
    }

    /// Parse all Python files recursively in src_root. Returns map of rel_path → entry.
    pub fn parse_all_files(&self) -> Result<HashMap<String, CacheEntry>> {
        // Walk src_root recursively, collect .py files, parse each.
        let mut result = HashMap::new();
        for entry in walkdir::WalkDir::new(&self.src_root) {
            let entry = entry?;
            if entry.file_name().to_string_lossy().ends_with(".py") {
                let rel_path = entry.path().strip_prefix(&self.src_root)?;
                let entry = self.parse_file(&rel_path.to_string_lossy())?;
                result.insert(rel_path.to_string_lossy().to_string(), *entry);
            }
        }
        Ok(result)
    }
}

impl Clone for RepoContext {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            src_root: self.src_root.clone(),
            parse_cache: Arc::clone(&self.parse_cache),
            git_repo: self.git_repo.clone(),
        }
    }
}

/// Parse all Python files in a repo into import graph edges.
pub fn build_import_graph(ctx: &RepoContext) -> Result<(Vec<Node>, Vec<GraphEdge>)> {
    let files = ctx.parse_all_files()?;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for (rel_path, entry) in &files {
        let node_id = nodes.len();
        nodes.push(rustpython_graph::Node {
            name: entry.name.clone(),
            path: rel_path.clone(),
        });
        for imp in &entry.imports {
            if let Some(target_idx) = nodes.iter().position(|n| n.name == imp.name) {
                edges.push(GraphEdge {
                    from: node_id,
                    to: target_idx,
                });
            } else {
                // Unknown module — skip edge or add stub.
            }
        }
    }

    Ok((nodes, edges))
}
```

**Context Design Notes:**

1. **Parse-once cache**: First call to `parse_file()` for a given path builds AST + imports + defs + comments. Subsequent calls hit the index map by relative path string.

2. **AST ownership**: All AST nodes borrow from `CacheEntry`. No AST cloning — reference via `&CacheEntry`.

3. **Git lazy init**: Git repository handle is only opened on first use of git-based tools. If git2 feature is absent or repo invalid, `git_repo` is `None`.

4. **Module-level cache**: Per-file AST cache maps `PathBuf → ModuleCacheEntry`. Key is relative to `src_root`.

### 4. Error Handling — `src/error.rs`

```rust
use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("Parse error in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: rustpython_parser::error::ParseError,
    },

    #[error("Not a valid git repository: {0}")]
    NotAGitRepository(PathBuf),

    #[error("No Python source found in {path}")]
    NoPythonSource(PathBuf),

    #[error("Graph analysis error: {0}")]
    GraphAnalysis(String),

    #[error("Context error: {0}")]
    Context(String),

    #[error("Tool validation error: {0}")]
    Validation(String),
}
```

### 5. Response Formatting — `src/fmt.rs`

Common response structures for tools:

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub path: String,
    pub line: usize,
    pub module: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub enum SymbolKind {
    Function,
    Class,
    Variable,
    Import,
    Decorator,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct CodeRankRow {
    pub module_name: String,
    pub module_score: f64,
    pub node_score: f64,
    pub edge_score: f64,
    pub import_count: usize,
}

impl CodeRankRow {
    pub fn display_score(&self, score: f64) -> f64 {
        (score * 1000.0).round() / 1000.0
    }
}
```

### 6. Router Composition — `src/router.rs`

```rust
use rmcp::Handler;
use crate::context::SharedContext;
use crate::tools::{search, code_rank, trace, analysis, git, config};

pub struct AppHandler {
    ctx: SharedContext,
}

impl rmcp::RouterExt for AppHandler {
    fn router(&self) -> rmcp::Router<Self> {
        rmcp::Router::new()
            .tool(search::ContextualSearchTool::new(&self.ctx))
            .tool(search::SmartCodeSearchTool::new(&self.ctx))
            .tool(search::ApiUsageTool::new(&self.ctx))
            .tool(code_rank::CodeRankAnalysisTool::new(&self.ctx))
            .tool(code_rank::CodeHotspotsTool::new(&self.ctx))
            .tool(trace::TraceDataFlowTool::new(&self.ctx))
            .tool(trace::TraceFeatureImplTool::new(&self.ctx))
            .tool(trace::TraceDependencyImpactTool::new(&self.ctx))
            .tool(analysis::ErrorPatternsTool::new(&self.ctx))
            .tool(analysis::PerfBottleneckTool::new(&self.ctx))
            .tool(analysis::ExecutionPathsTool::new(&self.ctx))
            .tool(git::RepoAnalysisTool::new(&self.ctx))
            .tool(git::RepoSymbolsTool::new(&self.ctx))
            .tool(config::GetConfigTool::new(&self.ctx))
            .tool(config::SetConfigTool::new(&self.ctx))
    }
}

impl AppHandler {
    pub fn new(ctx: SharedContext) -> Self {
        Self { ctx }
    }
}
```

### 7. Tool Modules

#### `src/tools/search/search.rs`

- **`contextual_search(keyword, repo_path, working_directory, num_context_lines)`**
  - Wrapper around `grep -r` for keyword search with context lines
  - Returns filepath, line number, matched line, context lines above/below

- **`smart_code_search(keyword, repo_path, working_directory, rank_results, context_lines, max_results)`**
  - Full implementation: builds import graph → CodeRank → filters results by importance score
  - Returns matches sorted by module rank (high-importance modules first)

- **`api_usage_examples(api_name, repo_path, working_directory, max_examples, group_by_pattern, include_tests, context_lines)`**
  - Finds all usages of `api_name` across `.py` files
  - Groups by code pattern (if `group_by_pattern = true`)
  - Returns categorized usage examples with surrounding context

#### `src/tools/code_rank/coderank.rs`

- **`coderank_analysis(repo_path, external_modules, top_n, min_connections, algorithm)`**
  - Builds Python import graph across `src_root/**/*.py`
  - Computes bidirectional weighted PageRank using `rustpython_graph::pagerank()`
  - Ranks modules by importance score, returns top `n`
  - Includes edge case handling:
    - `external_modules` filtering (e.g., google, genai, langchain)
    - `min_connections` threshold for graph pruning
    - Algorithm variants: `"bidirectional"`, `"import_weighted"`, `"all"`

- **`code_hotspots(repo_path, working_directory, min_connections, include_external, top_n)`**
  - Merges CodeRank scores with usage frequency
  - Returns modules ranked by: `importance_score = coderank_score * log(usage_freq + 1)`
  - Identifies modules that are both deeply connected and frequently called

#### `src/tools/trace/`

- **`trace_data_flow(data_identifier, repo_path, working_directory, max_depth, include_transformations, show_side_effects)`**
  - Finds all references to `data_identifier` (e.g., `"user_id"`, `"email"`)
  - Traces data flow by following imports and function calls
  - Returns flow path: source → transformers → destination
  - Optionally includes data transformation points

- **`trace_feature_implementation(feature_keywords, repo_path, working_directory, file_categories, include_tests, include_config, trace_depth)`**
  - Maps feature across layers: UI → API → Service → Data
  - Uses `file_categories` mapping to classify files (ui_frontend, api_controllers, business_logic, data_models, utils, tests)
  - Returns feature component map with file paths and line numbers

- **`trace_dependency_impact(target_module, repo_path, working_directory, analysis_type, max_depth, change_type)`**
  - Computes forward dependencies (what depends on this module) + backward dependencies (what this module depends on)
  - `analysis_type`: `"forward"`, `"backward"`, or `"both"`
  - `change_type`: `"modify"`, `"split"`, or `"remove"`

#### `src/tools/analysis/`

- **`error_patterns(repo_path, working_directory)`**
  - Scans all `.py` files for error handling patterns
  - Categorizes: `try/except`, `raise`, `Result<T, E>`, `panic!`
  - Reports consistency, missing error handling, exception chaining

- **`perf_bottleneck_analysis(repo_path, working_directory)`**
  - Detects: nested loops, repeated regex compilation, missing caching, large string concatenation in loops
  - Returns ranked bottlenecks with file, line, severity, and recommendation

- **`execution_paths(func_name, repo_path, working_directory, max_depth)`**
  - Maps decision points and branching logic in `func_name`
  - Follows imports and type aliases to resolve conditional branches
  - Returns execution path tree

#### `src/tools/git/`

- **`repo_analysis(repo_path, working_directory)`**
  - Uses git2 crate to query repo structure
  - Returns: total files, total size, commit count, top contributors, top modified files, last commit stats, branch list, file type breakdown

- **`repot_symbols(repo_path, working_directory, keep_types, file_must_contain, file_must_not_contain)`**
  - Replaces Python subprocess calls to `kit symbols`
  - Uses regex/ast parser to map Python symbols
  - Returns: file, symbol type, symbol name, line number, signature, docstring type, docstring preview

#### `src/tools/config/`

- **`get_config()`** → Current configuration
- **`set_config(key, value)`** → Update configuration
- Simple key-value store

### 8. Module Call Graph

```
main.rs
├── context.rs (SharedContext) ───┐
├── server.rs ─────────────────────┤
├── router.rs ─────────────────────┤  ← All tools receive SharedContext reference
├── fmt.rs ────────────────────────┤
├── error.rs ──────────────────────┤
│
├── tools/
│   ├── search/
│   │   ├── contextual_search      → grep::Searcher
│   │   ├── smart_code_search      → grep::Searcher + code_rank module
│   │   └── api_usage              → grep::Searcher
│   ├── code_rank/
│   │   ├── CodeRankAnalysis       → rustpython_graph, build_import_graph
│   │   └── CodeHotspots           → rustpython_graph + usage frequency
│   ├── trace/
│   │   ├── DataFlow               → rustpython_ast (AST path traversal)
│   │   ├── FeatureImpl            → rustpython_ast (AST path traversal)
│   │   └── DependencyImpact       → rustpython_ast + grep
│   ├── analysis/
│   │   ├── ErrorPatterns          → rustpython_ast + regex
│   │   ├── PerfBottleneck         → rustpython_ast + regex
│   │   └── ExecutionPaths         → rustpython_ast
│   ├── git/
│   │   ├── RepoAnalysis           → git2
│   │   └── RepoSymbols            → rustpython_parser + regex
│   └── config/
│       ├── GetConfig              → serde config
│       └── SetConfig              → serde config
│
└── parser/
    ├── mod.rs                     → Module definition
    ├── imports.rs                 → Import/def extraction
    └── symbols.rs                 → Token stream & pattern matching
```

### 9. Import Graph Construction

All CodeRank-powered tools share a single import graph construction path:

```
for each .py file in src_root/**/*.py:
    parse with rustpython_parser
    extract from-imports and import statements
    map to nodes: (module_path, file_path)
    create directed edges: A→B if file A imports from B

Graph shape: DiGraph<NodeId, WeightedEdge>
```

### 10. Git Layer

The git-based tools (`repo_analysis`, `repot_symbols`) use git2 crate for:
- Commit history analysis
- Branch information
- Tag enumeration
- Diff generation between versions

### 11. Tool Router

```rust
#[rmcp::tool_router]
pub struct ServerRouter {
    // Search group: grep + CodeRank
    search: Group<SearchTools>,
    // Analysis group: CodeRank + usage frequency
    analysis: Group<AnalysisTools>,
    // Data flow group: graph traversal
    data_flow: Group<DataFlowTools>,
    // Git group: git2 introspection
    git_group: Group<GitTools>,
    // Config group: serde key-value store
    config: Group<ConfigTools>,
}
```

### 12. Transport

- Streamable HTTP via axum: `RUSTANK_TRANSPORT` env var
- STDIO mode: default (backward-compatible with Python search-tools-mcp transport)

### 13. Testing Strategy

- **Unit tests**: `#[cfg(test)]` modules in each tool file
  - Test parse_file() correctness
  - Test import graph construction
  - Error handling paths
- **Integration tests**: `tests/` directory
  - Load real repos via `tempfile::tempdir()`
  - Test all 12 tools end-to-end
  - Snapshot-based for deterministic outputs
- **No mocks**: All tests use real Python repos, real file I/O

### 14. Implementation Timeline

| Phase | Duration | Tasks |
|---|---|---|
| P0 — Foundation | ~1h | Cargo.toml, main.rs, context.rs, error.rs, fmt.rs, server.rs |
| P1 — Parser | ~1.5h | parser/imports.rs, parser/symbols.rs, parse_file, parse_all_files |
| P2 — Search tools | ~1.5h | contextual_search, smart_code_search, api_usage |
| P3 — CodeRank tools | ~1.5h | CodeRankAnalysis, CodeHotspots, import_graph building |
| P4 — Trace tools | ~1.5h | data_flow, feature_implementation, dependency_impact |
| P5 — Analysis tools | ~1h | error_patterns, perf_bottleneck, execution_paths |
| P6 — Git tools | ~1h | repo_analysis, repot_symbols |
| P7 — Config tools | ~30m | get_config, set_config |
| P8 — Tests | ~2h | Unit tests, integration tests |
| P9 — Polish | ~30m | Documentation, README, formatting |

### 15. Edge Cases & Error Handling

- **Invalid repo paths**: Return `NotFound(repo_path)` error
- **Parse failures**: Log warning, skip file, continue processing
- **No git repo**: Return `MissingGitMetadata` error
- **Empty repos**: Return empty collections (not errors)
- **Permission denied**: Return `AccessDenied` error with path details
- **Large repos**: Add timeout/memoization for expensive graph operations
