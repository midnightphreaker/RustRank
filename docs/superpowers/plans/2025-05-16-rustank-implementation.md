# RustRank MCP Server Implementation Plan

## Goal

Build the `rustank` Rust MCP server with all 14 tools defined in DESIGN.md, using the rmcp 1.7.0 procedural-macro-based API and rustpython-parser 0.4.0.

## Architecture

**Single binary crate** `rustank` at `packages/rustank/`. Tools are grouped by feature into modules, composed via `#[tool_router]`. The `Context` struct (parse-once AST cache + optional git2 handle) is shared across all tool handlers.

**Transport:** Streamable HTTP over `axum`, falling back to STDIO. The rmcp 1.7.0 API uses:
- `#[tool(name = "...")]` procedural macro per handler
- `#[tool_router]` on a Router struct with `#[tool(...)]` method attributes
- `Router::build()` → `Server::new()` → `serve_server()` → StreamableHTTP transport

## Tech Stack (all embedded libraries, no subprocesses)

| Crate | Version | Purpose |
|---|---|---|
| `rmcp` | 1.7.0 | MCP server framework (procedural macros) |
| `rustpython-parser` | 0.4.0 | Python AST (parse + extract imports/defs) |
| `graphops` | 0.4.1 | Bidirectional PageRank (CodeRank core) |
| `grep` | 0.3.x | ripgrep engine (keyword search) |
| `git2` | 0.20 | Git operations (branch history, blame, merge-base) |
| `axum` | 0.8 | HTTP transport layer |
| `tokio` | 1.x | Async runtime |
| `serde`/`serde_json` | 1.x | Serialization |
| `walkdir` | 2.x | Recursive directory walk |
| `indexmap` | 2.x | Ordered collections |
| `thiserror` | 2.x | Error types |
| `anyhow` | 1.x | Fallible operations |

## File Mapping

| File | Responsibility |
|---|---|
| `Cargo.toml` (root) | Workspace root with `members = ["packages/rustank"]` |
| `packages/rustank/Cargo.toml` | Crate manifest, all dependencies |
| `packages/rustank/src/main.rs` | Entry point, `mod` declarations, transport startup |
| `packages/rustank/src/error.rs` | `AppError` enum (8 variants) + `Result<T>` |
| `packages/rustank/src/context.rs` | Parsing cache, source file walk, git lazy-open |
| `packages/rustank/src/fmt.rs` | `TableRow`, `CodeRankRow`, `HotspotRow` output types |
| `packages/rustank/src/tools/mod.rs` | Module + router assembly + `ALL_TOOLS` |
| `packages/rustank/src/tools/search.rs` | `contextual_search`, `smart_code_search`, `api_usage` |
| `packages/rustank/src/tools/code_rank.rs` | `coderank_analysis`, `code_hotspots` |
| `packages/rustank/src/tools/trace.rs` | `trace_data_flow`, `trace_feature_impl`, `trace_dep_impact` |
| `packages/rustank/src/tools/analysis.rs` | `error_patterns`, `perf_bottleneck`, `exec_paths` |
| `packages/rustank/src/tools/config.rs` | `get_config`, `set_config` (JSON env file) |
| `packages/rustank/tests/fixtures.rs` | Fixture management (copy real repos to tempdirs) |
| `packages/rustank/tests/integration.rs` | 14 integration tests (1 fixture × 14 tools) |

---

## Task Sequence

```
P0: Foundation   (Tasks 1-4): workspace, error.rs, context.rs, router.rs
P1: Search tools (Tasks 5-7): contextual_search, smart_code_search, api_usage
P2: CodeRank     (Tasks 8-9): coderank_analysis, code_hotspots
P3: Trace tools  (Tasks 10-12): trace_data_flow, trace_feature_impl, trace_dep_impact
P4: Analysis     (Tasks 13-15): error_patterns, perf_bottleneck, exec_paths
P5: Config       (Tasks 16-17): get_config, set_config
P6: Polish       (Tasks 18-20): tests, docs, CI config
```

---

### Task 1: Workspace Cargo.toml

**File:** `Cargo.toml`

**Steps:**
1. Create workspace Cargo.toml with `members = ["packages/rustank"]`, `resolver = "2"`
2. **Verify:** `cargo metadata --no-deps --format-version=1 | jq '.workspace_members'` outputs `["rustank 0.1.0 ..."]`
3. **Acceptance:** `cargo check --workspace` passes from repo root

---

### Task 2: Crate Manifest (`packages/rustank/Cargo.toml`)

**File:** `packages/rustank/Cargo.toml`

**Steps:**
1. Create `packages/rustank/` directory structure
2. Write Cargo.toml with all dependencies listed above. `git2` is **non-optional** (lazy-loaded at runtime, but always compiled).
3. **Verify:** `cargo check -p rustank` compiles with correct dependency versions
4. **Acceptance:** All crates resolve, no warnings about unused/cyclic deps

---

### Task 3: Error Module (`error.rs`)

**File:** `packages/rustank/src/error.rs`

**Error Enum (8 variants, in order of frequency):**
```rust
#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("Parse error in {path:?}: {source}")]
    Parse { path: PathBuf, #[source] source: rustpython_parser::error::ParseError },
    #[error("Not a git repository: {0}")]
    NotAGit(PathBuf),
    #[error("No Python source found in {0}")]
    NoPythonSource(PathBuf),
    #[error("Graph error: {0}")]
    Graph(String),
    #[error("Context error: {0}")]
    Context(String),
    #[error("Validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
```

**Steps:**
1. Write error.rs with enum above
2. Add to main.rs: `mod error;`
3. Write 3 unit tests: `graph` string contains label, `parse` includes path, `not_a_git` includes path
4. **Verify:** `cargo check -p rustank` passes (no `git2` feature conflict)
5. **Acceptance:** All 3 error tests pass

---

### Task 4: Context Module + Parse-Once Cache

**File:** `packages/rustank/src/context.rs`

**Structs:**

```rust
pub struct ModuleDef {
    pub path: PathBuf,
    pub imports: Vec<Import>,
    pub defs: Vec<Definition>,
    pub source_lines: Vec<String>,  // For line-based access
}

pub struct Import {
    pub module: String,      // "serde" or "serde::Serialize"
    pub name: Option<String>, // "Serialize" (from X import Y)
    pub line: usize,
}

pub struct Definition {
    pub name: String,
    pub kind: DefKind,  // Func, Struct, Class, Trait, Enum
    pub line: usize,
    pub end_line: usize,
    pub has_args: bool,
}
```

**Context:**
```rust
pub struct Context {
    path: PathBuf,                    // Repo root
    pub(crate) parse_cache: Arc<Mutex<HashMap<String, ModuleDef>>>,
    git_repo: OnceCell<Repository>,   // Lazy open
}
```

**Key methods:**
- `pub fn new(path: PathBuf) -> Self` — opens git repo lazily via `OnceCell`
- `pub fn parse_all(&self) -> Result<Vec<ModuleDef>>` — walk dirs, parse each `.py`
- `pub fn get_or_parse(&self, rel_path: String) -> Result<ModuleDef>` — parse-once: check cache → if miss, read + parse + cache + return

**rustpython-parser 0.4.0 API:**
- `rustpython_parser::parse_module(source, SourceMode::Module)` → `Result<Mod, ParseError>`
- `Mod::Module { body }` — `body: Vec<Stmt>`
- Statement kinds extracted during walk: `Stmt::Import`, `Stmt::FromImport`, `Stmt::FunctionDef`, `Stmt::AsyncFunctionDef`, `Stmt::ClassDef`
- `FunctionDef { name: Identifier, args: Arguments, body, lineno, end_lineno, async }`
- `Identifier { name }` — the `name` field is a `String`
- `Arguments { pos_args, kwonly_args, vararg, kwarg, defaults }`

**parse_all extraction:**
- For `Import`: `module = alias.name`, `name = alias.name`, `line = lineno`
- For `FromImport`: `module = from.module`, `name = alias.name`, `line = lineno`
- For `FunctionDef`: `name = func.name`, `args = count(func.args.pos_args)`, `line = func.lineno`, `end_line = func.end_lineno`
- For `ClassDef`: `name = class.name`, `kind = if bases { Trait } else { Struct }`

**Steps:**
1. Write context.rs with structs + Context impl
2. Write `extract_imports(&Mod) -> Vec<Import>` and `extract_defs(&Mod) -> Vec<Definition>`
3. Add `mod context;` in main.rs
4. **Compile check** — fix any type mismatches with rustpython-parser 0.4.0 API
5. Write 1 test: parse a minimal `.py` string with 1 import + 1 function, verify exports populated
6. **Acceptance:** parse-once cache works (first call parses, second returns cached clone)

---

### Task 5: Search Module (`tools/search.rs`)

**Tools:** `contextual_search`, `smart_code_search`, `api_usage`

**Shared implementation patterns:**
- `contextual_search`: Uses `grep::Searcher` (embedded ripgrep) for regex/file-glob text search. Returns matched lines with file path, line number, snippet.
- `smart_code_search`: Combines grep text search with CodeRank scoring. Import `coderank` from `analysis.rs` (deferred) or compute on-demand via git clone analysis.
- `api_usage`: Given an API name (e.g., `Error::from`), uses grep to find all usages, filters by file path, returns first N examples grouped by pattern.

**Parameters:**
- `contextual_search(path, pattern, file_type, is_regex, num_context_lines)`
- `smart_code_search(repo_path, pattern, context_lines, num_context_lines)`
- `api_usage(repo_path, api_name, max_examples, group_by_pattern)`

**Output:** JSON arrays of `{file, line, snippet, context_before, context_after}` for search; `{file, line, snippet, pattern_key}` for API usage.

**Steps:**
1. Write `contextual_search` handler using `grep::SearcherBuilder` + `regex::RegexSet`
2. Write `smart_code_search` handler (same as contextual_search, but with future CodeRank weighting placeholder)
3. Write `api_usage` handler using `grep` search for API name patterns
4. **Acceptance:** Each tool compiles, returns valid JSON results

---

### Task 6: CodeRank Module (`tools/code_rank.rs`)

**Tools:** `coderank_analysis`, `code_hotspots`

**Implementation:**
- `coderank_analysis`: Build import graph from parsed ASTs (each ModuleDef's imports → edges between modules). Run bidirectional PageRank via `graphops::pagerank(&graph, damping_factor)`. Return top-N modules ranked by PageRank score.
- `code_hotspots`: Cross-reference CodeRank scores with commit frequency (using `git2` blame/hist). Hotspot = high CR + high change frequency.

**Params:**
- `coderank_analysis(repo_path, top_n, module_prefix, external_modules)`
- `code_hotspots(repo_path, top_n, min_connections)`

**Steps:**
1. Build import graph from `Vec<ModuleDef>`: for each module, edges to its imported modules
2. Run `graphops::pagerank()` on the graph
3. Return top-N as `CodeRankRow { module, score, imports, depth }`
4. **Acceptance:** Run on a test fixture (see Task 18), verify scores sum to ~1.0

---

### Task 7: Trace Module (`tools/trace.rs`)

**Tools:** `trace_data_flow`, `trace_feature_impl`, `trace_dep_impact`

**trace_data_flow:**
- Given a data identifier (e.g., `user_id`, `email`), trace through AST to find all definitions, usages, and transformations
- Walk AST from parse cache, find references to identifier in variable names, function parameters, return values
- Follow import chains to track data across modules

**trace_feature_impl:**
- Given feature keywords (e.g., `["login", "authenticate"]`), map all code across layers:
  - UI/frontend: search for pattern in template/component files
  - API/controllers: search for endpoint patterns
  - Business logic: search for service/file patterns
  - Data: search for model/entity/schema patterns
- Use `grep` + AST analysis to find matching files, group by layer category

**trace_dep_impact:**
- Given a target module, find all files referencing it (import chain)
- Use `grep::Searcher` to find all usages of module name across the codebase
- Return dependency chain: direct imports → transitive references

**Params:**
- `trace_data_flow(repo_path, identifier, include_transformations, include_side_effects)`
- `trace_feature_impl(repo_path, feature_keywords)`
- `trace_dep_impact(repo_path, target_module)`

**Steps:**
1. Implement `trace_data_flow` using AST variable/identifier references
2. Implement `trace_feature_impl` using grep + layer categorization
3. Implement `trace_dep_impact` using import graph traversal
4. **Acceptance:** Each tool returns structured JSON output

---

### Task 8: Analysis Module (`tools/analysis.rs`)

**Tools:** `error_patterns`, `perf_bottleneck`, `exec_paths`

**error_patterns:**
- Use `grep::Searcher` (with embedded ripgrep) to find try-catch blocks
- Categorize errors by pattern (panic, Result, custom errors, unwrap)
- Check for consistency (all same pattern), detect anti-patterns (unwrap, panic on expected failures)
- Show evolution by analyzing git blame history for changed files

**perf_bottleneck:**
- Use `grep::Searcher` with patterns: `sleep`, `Thread::sleep`, `for` loops with range, `Vec::push` in loops
- Weight by code frequency/import graph
- Return ranked bottlenecks

**exec_paths:**
- Use syntax tree analysis to trace control flow through function calls
- Extract function signatures, conditionals, branches
- Use `grep` + AST to identify complex paths

**Params:**
- `error_patterns(repo_path, include_antipatterns, show_evolution, days_back)`
- `perf_bottleneck(repo_path, focus_areas, include_utility)`
- `execute_paths(repo_path, function_name, max_depth, include_call_contexts)`

**Steps:**
1. Implement `error_patterns`
2. Implement `perf_bottleneck`  
3. Implement `execute_paths` (exec_paths)
4. **Acceptance:** All return consistent JSON output

---

### Task 9: Config Module (`tools/config.rs`)

**Tools:** `get_config`, `set_config`

**Implementation:**
- Uses a JSON file (`.rustank_config.json`) as a local configuration store
- `get_config`: reads JSON, returns all keys
- `set_config`: writes JSON, returns updated config

**Steps:**
1. Implement file-based JSON storage
2. Write `get_config` and `set_config` handlers
3. **Acceptance:** Round-trip test — set config, get config, verify match

---

### Task 10: Router Assembly (`tools/mod.rs`)

**File:** `packages/rustank/src/tools/mod.rs`

**Structure:**
```rust
pub mod search;
pub mod code_rank;
pub mod trace;
pub mod analysis;
pub mod config;

use rmcp::serve::tool_router;
use search::*;
use code_rank::*;
use trace::*;
use analysis::*;
use config::*;

pub struct RustankRouter {
    // Router struct with tool handlers
}

#[tool_router]
impl RustankRouter {
    async fn contextual_search(&self, ...) -> ToolResult<...> { contextual_search_impl(...) }
    async fn smart_code_search(&self, ...) -> ToolResult<...> { smart_code_search_impl(...) }
    // ... 12 more tools
}
```

**Steps:**
1. Create module structure
2. Register each tool via `#[tool]` attribute
3. Build router with `#[tool_router]`
4. **Acceptance:** `cargo check` passes, router compiles

---

### Task 11: Server Entry Point (`main.rs`)

**File:** `packages/rustank/src/main.rs`

**Structure:**
```rust
mod error;
mod context;
mod tools;

use tools::RustankRouter;
use rmcp::serve::server::ServerBuilder;
use tokio::runtime::Runtime;

fn main() {
    let rt = Runtime::new().unwrap();
    let router = RustankRouter::new();
    // Configure server + transport
    rmcp::serve(&router, transport_config)?;
}
```

**Steps:**
1. Set up `main.rs` with `mod` declarations
2. Create router, configure transport
3. **Acceptance:** Server starts, responds to health checks

---

### Task 12: Test Fixtures

**File:** `packages/rustank/tests/fixtures.rs`

**Approach:**
- Use real repos from `tests/fixtures/` directory (git clone them during test setup)
- Copy to temporary directories, run tools against them
- Use `tempfile` crate for tempdir management

**Fixture list:**
- A small Rust project with 5-10 `.rs` files and imports
- A small Python project (use rustc/rust-analyzer repo snapshot)

**Steps:**
1. Create `tests/fixtures/` directory
2. Add fixture setup script
3. **Acceptance:** Fixtures can be built/loaded without network

---

### Task 13: Integration Tests

**File:** `packages/rustank/tests/integration.rs`

**Tests (14 tools × 1 fixture = 14 tests):**
1. `test_contextual_search_simple` — regex search with 1 match
2. `test_contextual_search_no_match` — regex search with 0 matches
3. `test_smart_code_search_ranked` — search results ordered by importance
4. `test_api_usage_example` — find 1 API usage
5. `test_coderank_imports` — top module has highest import count
6. `test_code_hotspot_detection` — module appearing in many imports is hotspot
7. `test_trace_data_flow` — trace identifier through AST
8. `test_trace_feature_impl_single` — feature keyword finds matching file
9. `test_trace_dep_impact_single` — module import traced to dependent
10. `test_error_patterns_found` — detects at least 1 error pattern
11. `test_perf_bottleneck_detected` — detects sleep in loop
12. `test_exec_paths_simple` — finds conditional branches
13. `test_get_config_default` — returns empty/null defaults
14. `test_set_config_roundtrip` — set → get round-trips correctly

**Acceptance:** All 14 tests pass locally

---

### Task 14: Polish

**Steps:**
1. Run `cargo fmt` and `cargo clippy --all-targets --all-features`
2. Fix all warnings/clippy lints
3. Add minimal README.md
4. Add `.gitignore` for build artifacts
5. **Acceptance:** Zero warnings, zero clippy lints

---

## Self-Review Checklist

Before presenting this plan for approval:
- [ ] **Placeholder scan:** No `// implement later` or `// TBD` in tools module files
- [ ] **Consistency check:** Error types match between error.rs and each tool handler
- [ ] **Scope check:** 14 tools only, no extras, no missing tools from DESIGN.md
- [ ] **API alignment:** rmcp 1.7.0 `#[tool]`/`#[tool_router]` pattern consistent
- [ ] **Dependency alignment:** All deps embedded, no subprocess calls
