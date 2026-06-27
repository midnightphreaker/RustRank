# RustRank Parallel Remaining Risks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the remaining RustRank robustness risks with parallel subagents where workstreams are independent, while requiring each subagent to load exact archived skills before work.

**Architecture:** Split the work into three independent implementation lanes: call graph correctness, lazy Python recovery spans, and embedding request bounding. Each lane owns a disjoint primary module plus focused integration tests, then the coordinator integrates conflicts, runs full verification, and dispatches a final review subagent.

**Tech Stack:** Rust 2024, tree-sitter parsers, rustpython-parser fallback recovery, serde JSON manifests, rmcp 1.7, standard Rust integration tests, archived Codex skills under `~/.codex/skills-archive`.

---

## File Structure

- `src/src/process.rs`: Owns syntactic call extraction, qualified internal call keys, process call-chain derivation, and `CallEdge`.
- `src/src/tools/agent.rs`: Owns `RepoGraph`, `symbol_context`, `impact`, `query`, and resource consumers of call/process data.
- `src/src/index.rs`: Owns manifest `CALLS` graph edge serialization and must use qualified `CallEdge` keys without changing public JSON shape.
- `src/src/context.rs`: Owns lazy Python fallback parsing and recovered definition span assignment.
- `src/src/embeddings.rs`: Owns embedding cache reads, request execution, per-index request limit, and warnings.
- `src/tests/integration.rs`: Owns all new regression tests for call graph quality, Python fallback spans, and embedding request cap.
- `scripts/smoke_http_json.py`: No planned change unless the final review finds a smoke coverage gap.
- `docs/superpowers/plans/2026-06-27-parallel-remaining-risks.md`: This implementation plan.

## Required Coordinator Setup

- [ ] **Step 1: Run startup health check**

Run:

```bash
test -d "$HOME/.codex/skills" && test -d "$HOME/.codex/skills/download-skill" && test -f "$HOME/.codex/skills/download-skill/SKILL.md" && test -d "$HOME/.codex/skills/find-skill" && test -f "$HOME/.codex/skills/find-skill/SKILL.md" && test -d "$HOME/.codex/skills/load-skill" && test -f "$HOME/.codex/skills/load-skill/SKILL.md" && test -d "$HOME/.codex/skills-archive" && test -d "$HOME/.codex/skills-archive/download-skill" && test -f "$HOME/.codex/skills-archive/download-skill/SKILL.md" && test -d "$HOME/.codex/skills-archive/find-skill" && test -f "$HOME/.codex/skills-archive/find-skill/SKILL.md" && test -d "$HOME/.codex/skills-archive/load-skill" && test -f "$HOME/.codex/skills-archive/load-skill/SKILL.md"
```

Expected: exit code `0`.

- [ ] **Step 2: Confirm exact archived skills exist**

Run:

```bash
for name in rust-review rust-testing rust-async code-reviewer test-driven-development vibe-code-auditor performance-testing-review-ai-review; do
  test -f "$HOME/.codex/skills-archive/$name/SKILL.md" || { echo "missing $name"; exit 1; }
done
```

Expected: no output and exit code `0`.

- [ ] **Step 3: Record current dirty worktree without reverting it**

Run:

```bash
git status --short
```

Expected: existing RustRank parity changes may be present. Do not revert them.

## Parallel Batch A: Dispatch Three Implementation Workers

These workers can run in parallel because each has a distinct primary write set. They all may edit `src/tests/integration.rs`; the coordinator must integrate that shared test file after all workers return.

### Task 1: Call Graph Correctness Worker

**Files:**
- Modify: `src/src/process.rs`
- Modify: `src/src/tools/agent.rs`
- Modify: `src/src/index.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Dispatch worker with exact instructions**

Use this subagent prompt:

```markdown
Role: Rust call graph correctness worker.

Objective: Replace false-positive text call detection with syntax-node call extraction and qualified internal call keys, while preserving public output shapes.

Primary write ownership:
- `src/src/process.rs`
- `src/src/tools/agent.rs`
- `src/src/index.rs`

Shared test write:
- `src/tests/integration.rs`

Forbidden:
- Do not modify `src/src/embeddings.rs`.
- Do not modify `src/src/context.rs` except to make an existing Tree-sitter language helper `pub(crate)` if absolutely needed.
- Do not commit.
- Do not revert user or other-agent changes.

## Required archived skills to load

The subagent MUST load every non-`NONE` skill below with `load-skill` before reading additional project context or starting work.

1. `rust-review`
2. `rust-testing`
3. `code-reviewer`
4. `test-driven-development`
5. `vibe-code-auditor`

Required load step:
- [ ] Run `load-skill "rust-review,rust-testing,code-reviewer,test-driven-development,vibe-code-auditor"`.
- [ ] Confirm each selected skill loaded.
- [ ] Stop and report `BLOCKED` if any selected skill cannot be loaded.

Implementation requirements:
1. Add failing tests first:
   - `call_edges_ignore_comment_and_string_call_lookalikes`
   - `call_edges_do_not_cross_duplicate_bare_symbol_names`
   - `agent_context_uses_call_edges_not_raw_substrings`
2. Replace `process.rs` raw regex line scanning with Tree-sitter call-expression extraction.
3. Add internal qualified keys to `CallEdge`:

```rust
pub source_key: String,
pub target_key: String,
```

4. Keep existing display fields such as `source_symbol`, `target_symbol`, files, modules, and lines.
5. Update `derive_processes` grouping to use `source_key`.
6. Update `index.rs` manifest `CALLS` graph edge generation to use `edge.source_key` and `edge.target_key`.
7. Update `RepoGraph::callers_of` and `RepoGraph::calls_from` in `agent.rs` to use `call_edges()` instead of raw substring matching.
8. Preserve all public serialized shapes for `SymbolContext`, `ImpactReport`, `QueryResult`, `ProcessFlow`, `ProcessStep`, and manifest graph edge fields.

Suggested test fixture shape:

```rust
std::fs::write(
    dir.path().join("pkg/a.py"),
    r#"
def entry():
    return shared()

def noise():
    # shared()
    return "shared()"

def shared():
    return 1
"#,
)?;
std::fs::write(
    dir.path().join("pkg/b.py"),
    "def shared():\n    return 2\n",
)?;
std::fs::write(
    dir.path().join("src/lib.rs"),
    r#"
pub fn entry_rs() {
    shared();
}

pub fn noise_rs() {
    // shared()
    let _s = "shared()";
}

pub fn shared() {}
"#,
)?;
```

Expected assertions:
- No `CALLS` edge is emitted for comment/string-only lines.
- Python `entry` calls only `symbol:python:pkg/a.py:shared`.
- Python `entry` does not call `pkg/b.py:shared` or Rust `shared`.
- `symbol_context(repo, "entry").callees` excludes comment/string-only symbols.

Verification:
- Run `cargo test --workspace --all-features call_edges -- --nocapture`.
- Run `cargo test --workspace --all-features agent_context_uses_call_edges -- --nocapture`.

Handoff format:
- Result: PASS/PARTIAL/FAIL/BLOCKED
- Files changed
- Archived skills loaded
- Commands run and outputs summarized
- Important code paths changed
- Remaining risks
```

- [ ] **Step 2: Wait for worker only when integration is needed**

Expected worker result: changed `process.rs`, `agent.rs`, `index.rs`, and integration tests.

### Task 2: Lazy Python Recovery Span Worker

**Files:**
- Modify: `src/src/context.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Dispatch worker with exact instructions**

Use this subagent prompt:

```markdown
Role: Rust/Python parser recovery worker.

Objective: Improve lazy Python fallback `end_line` spans so recovered multi-line functions/classes include their bodies and downstream context/process/change detection sees body lines.

Primary write ownership:
- `src/src/context.rs`

Shared test write:
- `src/tests/integration.rs`

Forbidden:
- Do not modify `src/src/process.rs`, `src/src/tools/agent.rs`, `src/src/index.rs`, or `src/src/embeddings.rs`.
- Do not commit.
- Do not revert user or other-agent changes.

## Required archived skills to load

The subagent MUST load every non-`NONE` skill below with `load-skill` before reading additional project context or starting work.

1. `rust-review`
2. `rust-testing`
3. `code-reviewer`
4. `test-driven-development`
5. `NONE`

Required load step:
- [ ] Run `load-skill "rust-review,rust-testing,code-reviewer,test-driven-development"`.
- [ ] Confirm each selected skill loaded.
- [ ] Stop and report `BLOCKED` if any selected skill cannot be loaded.

Implementation requirements:
1. Add failing test `lazy_python_recovery_assigns_body_spans`.
2. In `context.rs`, change lazy fallback internals so recovered definitions retain header indentation.
3. Add deterministic end-line assignment:
   - Start at the definition header line.
   - Scan forward.
   - Ignore blank and comment-only lines for boundary decisions.
   - Stop before the first later nonblank/comment line whose indentation is less than or equal to the definition header indentation.
   - Otherwise extend to the last meaningful indented line before EOF.
4. Keep nested defs/classes as separate definitions. Parent spans may include nested defs.
5. Do not change RustPython AST parsing or Tree-sitter parsing paths.

Suggested test source:

```python
import os

def authenticate(user_id):
    user = build_user(user_id)
    return user

def build_user(user_id):
    return {"id": user_id}

class Recoverable:
    def member(self):
        return build_user("nested")

async def later(value):
    return value

if broken(
```

Expected assertions:
- `authenticate.line == 4`
- `authenticate.end_line == 6`
- `build_user.line == 8`
- `build_user.end_line == 9`
- `Recoverable.line == 11`
- `Recoverable.end_line == 13`
- `member.line == 12`
- `member.end_line == 13`
- `later.line == 15`
- `later.end_line == 16`

Verification:
- Run `cargo test --workspace --all-features lazy_python_recovery -- --nocapture`.

Handoff format:
- Result: PASS/PARTIAL/FAIL/BLOCKED
- Files changed
- Archived skills loaded
- Commands run and outputs summarized
- Important code paths changed
- Remaining risks
```

- [ ] **Step 2: Wait for worker only when integration is needed**

Expected worker result: changed `context.rs` and integration tests.

### Task 3: Embedding Request Cap Worker

**Files:**
- Modify: `src/src/embeddings.rs`
- Test: `src/tests/integration.rs`

- [ ] **Step 1: Dispatch worker with exact instructions**

Use this subagent prompt:

```markdown
Role: Rust embedding performance/reliability worker.

Objective: Bound embedding-enabled indexing when endpoints are slow or bad by limiting uncached embedding requests per index run, while preserving cache-hit behavior and public API shape.

Primary write ownership:
- `src/src/embeddings.rs`

Shared test write:
- `src/tests/integration.rs`

Forbidden:
- Do not modify `src/src/process.rs`, `src/src/context.rs`, `src/src/tools/agent.rs`, or `src/src/index.rs`.
- Do not commit.
- Do not revert user or other-agent changes.

## Required archived skills to load

The subagent MUST load every non-`NONE` skill below with `load-skill` before reading additional project context or starting work.

1. `rust-review`
2. `rust-testing`
3. `rust-async`
4. `performance-testing-review-ai-review`
5. `code-reviewer`

Required load step:
- [ ] Run `load-skill "rust-review,rust-testing,rust-async,performance-testing-review-ai-review,code-reviewer"`.
- [ ] Confirm each selected skill loaded.
- [ ] Stop and report `BLOCKED` if any selected skill cannot be loaded.

Implementation requirements:
1. Add failing tests:
   - `embedding_index_caps_uncached_requests_without_failing_index`
   - `embedding_index_request_cap_ignores_cache_hits`
2. Add internal constant in `embeddings.rs`:

```rust
const MAX_EMBEDDING_REQUESTS_PER_INDEX: usize = 4;
```

3. In `index_embeddings`, preserve cache hit handling before applying the cap.
4. Count only uncached fetch attempts against the cap.
5. Once cap is reached, skip remaining uncached files and emit one summary warning:

```text
embedding indexing partial: request limit reached after 4 embedding requests; skipped N uncached source files; index_project succeeded with partial embeddings; rerun index_project to continue filling the embedding cache
```

6. Do not change `IndexProjectResponse`, `EmbeddingOptions`, or MCP request schemas.

Expected assertions:
- First full-fixture embedding run makes exactly 4 mock requests.
- First run indexes all source files despite partial embeddings.
- First run writes exactly 4 embedding cache JSON files.
- Warning contains `embedding indexing partial`, `request limit reached after 4 embedding requests`, `skipped`, and `partial embeddings`.
- Second run with existing cache makes 4 additional mock requests and grows cache from 4 to 8 files.

Verification:
- Run `cargo test --workspace --all-features embedding_index_caps -- --nocapture`.
- Run `cargo test --workspace --all-features embedding_index_request_cap -- --nocapture`.

Handoff format:
- Result: PASS/PARTIAL/FAIL/BLOCKED
- Files changed
- Archived skills loaded
- Commands run and outputs summarized
- Important code paths changed
- Remaining risks
```

- [ ] **Step 2: Wait for worker only when integration is needed**

Expected worker result: changed `embeddings.rs` and integration tests.

## Integration Task: Merge Parallel Results

**Files:**
- Modify: `src/tests/integration.rs`
- Review: `src/src/process.rs`
- Review: `src/src/tools/agent.rs`
- Review: `src/src/index.rs`
- Review: `src/src/context.rs`
- Review: `src/src/embeddings.rs`

- [ ] **Step 1: Inspect returned worker changes**

Run:

```bash
git status --short
git diff -- src/src/process.rs src/src/tools/agent.rs src/src/index.rs src/src/context.rs src/src/embeddings.rs src/tests/integration.rs
```

Expected: no unexpected files outside the declared scopes.

- [ ] **Step 2: Resolve shared `integration.rs` conflicts manually**

If multiple workers edited `src/tests/integration.rs`, keep all tests and helper functions. Expected test names:

```rust
call_edges_ignore_comment_and_string_call_lookalikes
call_edges_do_not_cross_duplicate_bare_symbol_names
agent_context_uses_call_edges_not_raw_substrings
lazy_python_recovery_assigns_body_spans
embedding_index_caps_uncached_requests_without_failing_index
embedding_index_request_cap_ignores_cache_hits
```

- [ ] **Step 3: Run focused tests**

Run:

```bash
cargo test --workspace --all-features call_edges -- --nocapture
cargo test --workspace --all-features agent_context_uses_call_edges -- --nocapture
cargo test --workspace --all-features lazy_python_recovery -- --nocapture
cargo test --workspace --all-features embedding_index_caps -- --nocapture
cargo test --workspace --all-features embedding_index_request_cap -- --nocapture
```

Expected: all PASS.

- [ ] **Step 4: Run formatting**

Run:

```bash
cargo fmt --all
cargo fmt --all -- --check
```

Expected: second command PASS.

## Parallel Batch B: Dispatch Review Subagents

These review subagents run after integration and before broad verification.

### Task 4: Call Graph Review Subagent

**Files:**
- Review: `src/src/process.rs`
- Review: `src/src/tools/agent.rs`
- Review: `src/src/index.rs`
- Review: `src/tests/integration.rs`

- [ ] **Step 1: Dispatch read-only reviewer**

Use this subagent prompt:

```markdown
Role: Rust call graph review subagent.

Objective: Review the integrated call graph changes for false positives, duplicate-symbol contamination, public shape regressions, and missing tests.

Allowed to modify: none.

## Required archived skills to load

1. `rust-review`
2. `rust-testing`
3. `code-reviewer`
4. `vibe-code-auditor`
5. `NONE`

Required load step:
- [ ] Run `load-skill "rust-review,rust-testing,code-reviewer,vibe-code-auditor"`.
- [ ] Confirm each selected skill loaded.
- [ ] Stop and report `BLOCKED` if any selected skill cannot be loaded.

Review:
- Confirm comments and strings do not produce CALLS.
- Confirm duplicate bare symbols do not fan out across modules/languages.
- Confirm manifest `GraphEdge` serialized shape is unchanged.
- Confirm `SymbolContext`, `ImpactReport`, `QueryResult`, `ProcessFlow`, and `ProcessStep` public fields are unchanged.

Return findings first, with file/line refs.
```

### Task 5: Parser and Embedding Review Subagent

**Files:**
- Review: `src/src/context.rs`
- Review: `src/src/embeddings.rs`
- Review: `src/tests/integration.rs`

- [ ] **Step 1: Dispatch read-only reviewer**

Use this subagent prompt:

```markdown
Role: Rust parser and embedding review subagent.

Objective: Review lazy Python span recovery and embedding request cap behavior for correctness, reliability, and test quality.

Allowed to modify: none.

## Required archived skills to load

1. `rust-review`
2. `rust-testing`
3. `rust-async`
4. `performance-testing-review-ai-review`
5. `code-reviewer`

Required load step:
- [ ] Run `load-skill "rust-review,rust-testing,rust-async,performance-testing-review-ai-review,code-reviewer"`.
- [ ] Confirm each selected skill loaded.
- [ ] Stop and report `BLOCKED` if any selected skill cannot be loaded.

Review:
- Confirm lazy Python fallback spans are deterministic and scoped only to fallback recovery.
- Confirm nested defs/classes behave as documented.
- Confirm cached embeddings do not count against the request cap.
- Confirm the partial embedding warning is one summary warning and does not leak source content or API keys.

Return findings first, with file/line refs.
```

## Final Verification Task

**Files:**
- All changed files

- [ ] **Step 1: Run full format check**

Run:

```bash
cargo fmt --all -- --check
```

Expected: PASS.

- [ ] **Step 2: Run full check**

Run:

```bash
cargo check --workspace --all-targets --all-features
```

Expected: PASS.

- [ ] **Step 3: Run clippy**

Run:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Run full tests**

Run:

```bash
cargo test --workspace --all-features
```

Expected: PASS.

- [ ] **Step 5: Run HTTP smoke**

Start server:

```bash
RUSTRANK_TRANSPORT=streamable_http RUSTRANK_HOST=127.0.0.1 RUSTRANK_PORT=63481 ./target/debug/rustrank
```

In another shell, run:

```bash
python3 scripts/smoke_http_json.py --url http://127.0.0.1:63481/mcp
```

Expected output includes:

```text
listed 19 tools
called 19 tools successfully
exercised resources/list, resources/templates/list, and resources/read
```

Stop the server with Ctrl-C.

- [ ] **Step 6: Refresh RustRank index**

Run:

```bash
./target/debug/rustrank index-project --repo-path /home/mp/.mcp-servers/RustRank --force-rebuild --clean-stale
```

Expected: JSON response with `indexed_files` equal to `scanned_files` and no unexpected warnings.

## Git and Reporting Rules

- [ ] **Step 1: Do not commit unless explicitly asked**

The writing-plans skill normally recommends commits, but repository instructions and developer instructions forbid committing unless the user explicitly asks. Use `git diff` checkpoints instead.

- [ ] **Step 2: Final response must include evidence**

Report:

```text
Status: complete / partial / blocked
Subagents used: names and tasks
Archived skills loaded by each subagent
Files changed
Checks/tests run
Key evidence
Remaining risks
```

## Self-Review

- Spec coverage: covers parallel subagent dispatch, exact skill packets, disjoint write ownership, integration, review, and verification.
- Placeholder scan: no placeholder markers remain.
- Type consistency: file paths, function names, test names, and skill names match current repository context and archived skill frontmatter.
- Scope check: implementation is split into three independently testable workstreams and two read-only review workstreams.
