# RustRank Implementation Notes

## Architecture

RustRank is a single Rust workspace member in `src/`. The binary entry point
delegates to `rustrank::tools::serve()`, which handles utility CLI commands,
stdio transport, and stateless no-SSE Streamable HTTP transport.

Core modules:

| Module | Responsibility |
| --- | --- |
| `context` | Source discovery, parsing, module definitions, import resolution |
| `project_config` | Raw config I/O and typed enabled-language lookup |
| `index` | Persistent per-language caches, project manifest generation, and AGENTS index summary updates |
| `tools::*` | MCP-facing request handlers and tool router |
| `fmt` | Shared tool output row types |
| `error` | `AppError` and crate `Result` |

## Language Handling

Language detection scans non-ignored files and counts extension matches. The
same matcher feeds parsing, enabled-language filtering, and indexing.

Enabled languages come from `.rustrank_config.json`:

```json
{
  "languages": {
    "enabled": ["python", "rust"]
  }
}
```

When no valid language is configured, RustRank auto-detects from source files.
Existing tools call `supported_source_files`, so config filtering applies to
parsing, CodeRank, trace, and analysis paths.

## Persistent Index

`index_project` and the `index-project` CLI command write deterministic JSON to:

```text
.rustrank/index/v1/
```

Indexing flow:

1. Resolve enabled languages from CLI/tool arguments or project config.
2. Walk supported source files, skipping ignored directories and unsupported
   extensions.
3. Compute a BLAKE3 hash for each source file.
4. Reuse a compatible per-file cache when the hash and cache header match.
5. Parse cache misses with the existing parser stack.
6. Write per-language `index.json` shards.
7. Remove stale hash files when `clean_stale` is true.
8. Build `project_manifest.json` with modules, resolved edges, and unresolved
   imports.
9. Create or amend `<repo_path>/AGENTS.md` with a generated
   `## RustRank Indexed Codebase` section bounded by
   `<!-- rustrank-index:start -->` and `<!-- rustrank-index:end -->`.

Cache files intentionally exclude source lines, snippets, absolute paths, and
timestamps. Existing tools may use a valid manifest for metadata-heavy work, but
must fall back to live parsing when cache state is stale.

## Public Interfaces

MCP tool:

```json
{
  "repo_path": "/path/to/repo",
  "languages": ["python", "rust"],
  "force_rebuild": false,
  "clean_stale": true
}
```

CLI:

```bash
cargo run -p rustrank -- index-project --repo-path /path/to/repo
cargo run -p rustrank -- index-project --repo-path /path/to/repo --languages python,rust --force-rebuild --clean-stale
```

Registered tools can be checked with:

```bash
cargo run -p rustrank -- --list-tools
```

## Verification

Use TDD for behavioral changes. Preferred checks before commit/push:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python3 -m py_compile scripts/smoke_http_json.py
cargo run -p rustrank -- --list-tools
```

For HTTP/Docker changes, also run the no-SSE smoke test and Docker build:

```bash
docker build -t rustrank:local .
scripts/smoke_http_json.py --url http://127.0.0.1:63477/mcp
```
