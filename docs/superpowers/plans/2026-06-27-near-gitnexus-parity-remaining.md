# RustRank Remaining Near GitNexus Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the remaining agent-facing parity items: real embedding cache/scoring, process-flow resources, and stronger MCP/API verification.

**Architecture:** Keep RustRank JSON-first. Add embedding and process helpers as focused modules, then integrate them through the existing `index_project`, `query`, MCP resources, and smoke tests without changing existing tool response shapes more than additive fields.

**Tech Stack:** Rust 2024, serde JSON caches, git2, rmcp 1.7, axum smoke server, standard-library HTTP-free tests with mockable local endpoints.

---

## File Structure

- `src/src/embeddings.rs`: embedding config, OpenAI-compatible request/response structs, cache read/write, cosine scoring, and mockable HTTP fetch.
- `src/src/tools/agent.rs`: `query` semantic-score integration and process resource rendering.
- `src/src/index.rs`: index-time embedding cache population and process data in manifest or companion JSON.
- `src/src/project_config.rs`: typed embedding config reader using existing dotted config support.
- `src/src/tools/mod.rs`: pass optional embedding request fields through to indexing.
- `src/tests/integration.rs`: fixture tests for embeddings, processes, resources, and backward-compatible calls.
- `scripts/smoke_http_json.py`: HTTP smoke coverage for resources, templates, embedding flags, and all tools.

## Task 1: Embedding Config and Cache

**Files:**
- Create: `src/src/embeddings.rs`
- Modify: `src/src/lib.rs`
- Modify: `src/src/project_config.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Write failing config/cache tests**

Add tests that set `embeddings.enabled`, `embeddings.model`, and `embeddings.dimensions`, then assert the typed config reads defaults and overrides. Add a cache write/read test using a fixed hash key and vector.

- [ ] **Step 2: Run focused test**

Run: `cargo test --workspace --all-features embeddings -- --nocapture`

Expected: FAIL because `embeddings` module/config helpers do not exist.

- [ ] **Step 3: Implement minimal config/cache**

Create `EmbeddingConfig` with defaults:

```rust
pub const DEFAULT_BASE_URL: &str = "https://api.phrk.org/v1";
pub const DEFAULT_MODEL: &str = "text-image-embedding";
pub const DEFAULT_DIMENSIONS: usize = 1536;
```

Read `.rustrank_config.json` via existing project config, support request overrides, and write cache JSON files under `.rustrank/index/v1/embeddings/<hash>.json`.

- [ ] **Step 4: Verify focused test passes**

Run: `cargo test --workspace --all-features embeddings -- --nocapture`

Expected: PASS.

## Task 2: Mockable OpenAI-Compatible Embeddings

**Files:**
- Modify: `src/src/embeddings.rs`
- Modify: `src/Cargo.toml` if a minimal HTTP client dependency is needed
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Write failing mock endpoint tests**

Start a local TCP listener in the test. Assert the request body includes `model`, `input`, and `dimensions`, and assert Authorization is omitted when no API key is provided.

- [ ] **Step 2: Run focused test**

Run: `cargo test --workspace --all-features embedding_endpoint -- --nocapture`

Expected: FAIL because fetch is not implemented.

- [ ] **Step 3: Implement HTTP fetch**

Implement a small POST client that sends to `{base_url}/embeddings`, parses `data[0].embedding`, validates vector dimensions, and never logs API keys.

- [ ] **Step 4: Verify focused test passes**

Run: `cargo test --workspace --all-features embedding_endpoint -- --nocapture`

Expected: PASS.

## Task 3: Index-Time Embedding Population

**Files:**
- Modify: `src/src/index.rs`
- Modify: `src/src/tools/mod.rs`
- Modify: `src/src/tools/index.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Write failing index tests**

Call `index_project` with embeddings enabled and a mock endpoint. Assert `.rustrank/index/v1/embeddings/` contains cache files and a second run reuses them.

- [ ] **Step 2: Run focused test**

Run: `cargo test --workspace --all-features index_project_embedding -- --nocapture`

Expected: FAIL because index-time embedding population is not wired.

- [ ] **Step 3: Wire indexing options**

Add an options struct while preserving existing `index_project(repo, languages, force, clean)` calls. Use a new internal function for the option-rich path.

- [ ] **Step 4: Verify focused test passes**

Run: `cargo test --workspace --all-features index_project_embedding -- --nocapture`

Expected: PASS.

## Task 4: Semantic Query Scoring

**Files:**
- Modify: `src/src/tools/agent.rs`
- Modify: `src/src/embeddings.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Write failing semantic ranking test**

Seed cached embeddings for two symbols and a query embedding. Assert `query` ranks the semantically closer symbol above a lexical-only competitor and includes a `semantic` match reason.

- [ ] **Step 2: Run focused test**

Run: `cargo test --workspace --all-features semantic_query -- --nocapture`

Expected: FAIL because query ignores embeddings.

- [ ] **Step 3: Add semantic scoring**

Load cached vectors when present, compute cosine similarity, add it to lexical and centrality scores, and keep lexical fallback unchanged when vectors are missing.

- [ ] **Step 4: Verify focused test passes**

Run: `cargo test --workspace --all-features semantic_query -- --nocapture`

Expected: PASS.

## Task 5: Process Flow Derivation

**Files:**
- Create: `src/src/process.rs` or add focused helpers in `src/src/tools/agent.rs`
- Modify: `src/src/lib.rs` if a new module is created
- Modify: `src/src/index.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Write failing process tests**

Assert login/authenticate fixture code produces at least one process with entry symbol, file, module, and call-chain nodes.

- [ ] **Step 2: Run focused test**

Run: `cargo test --workspace --all-features process -- --nocapture`

Expected: FAIL because process chains are currently shallow text summaries.

- [ ] **Step 3: Implement call-chain derivation**

Use parsed definitions plus `simple_calls` to build bounded call chains from entry-like symbols (`main`, exported/public functions, handler/controller/login/authenticate names).

- [ ] **Step 4: Verify focused test passes**

Run: `cargo test --workspace --all-features process -- --nocapture`

Expected: PASS.

## Task 6: Process Resources and Query Grouping

**Files:**
- Modify: `src/src/tools/agent.rs`
- Modify: `src/src/index.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Write failing resource tests**

Assert `rustrank://repo/current/processes` lists process names and `rustrank://repo/current/process/{name}` returns a readable call chain. Assert unknown process URI returns an error.

- [ ] **Step 2: Run focused test**

Run: `cargo test --workspace --all-features process_resource -- --nocapture`

Expected: FAIL until process resources use derived process data.

- [ ] **Step 3: Implement resources**

Render stable markdown. Add an optional `process` field to `QueryResult` only if needed; otherwise include `process:<name>` in `match_reasons`.

- [ ] **Step 4: Verify focused test passes**

Run: `cargo test --workspace --all-features process_resource -- --nocapture`

Expected: PASS.

## Task 7: MCP Smoke and Backward Compatibility

**Files:**
- Modify: `scripts/smoke_http_json.py`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Add smoke assertions**

Extend smoke to call `resources/list`, `resources/templates/list`, and `resources/read` for context/process resources. Add an `index_project` call with embedding flags but no API key.

- [ ] **Step 2: Run smoke**

Run server on a non-default port, then:

```bash
python3 scripts/smoke_http_json.py --url http://127.0.0.1:<port>/mcp
```

Expected: PASS, listed tools/resources and no SSE response.

## Task 8: Full Verification

**Files:**
- All changed files

- [ ] **Step 1: Format**

Run: `cargo fmt --all -- --check`

Expected: PASS.

- [ ] **Step 2: Check**

Run: `cargo check --workspace --all-targets --all-features`

Expected: PASS.

- [ ] **Step 3: Clippy**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: PASS.

- [ ] **Step 4: Tests**

Run: `cargo test --workspace --all-features`

Expected: PASS.

- [ ] **Step 5: HTTP smoke**

Run the smoke script against a local server and stop the server afterward.

Expected: PASS.

## Self-Review

- Spec coverage: covers remaining embedding cache/scoring, process resources, query grouping, compatibility, and smoke verification.
- Placeholder scan: no TBD/TODO placeholders.
- Type consistency: public names match existing `index_project`, `QueryResult`, and MCP resource naming.
