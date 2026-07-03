# RustRank

RustRank is a Rust MCP server and CLI for repository analysis. It indexes
source files into a deterministic repo-local cache, exposes graph-aware MCP
tools for agents, and can run over stdio or stateless Streamable HTTP.

Use RustRank when an agent or tool needs fast, repeatable answers about a code
repository: supported languages, modules, imports, symbols, graph context,
impact, feature traces, error patterns, and generated agent orientation.

## Quick start

These commands build RustRank, verify that the binary starts, and index the
current repository. The index command writes `.rustrank/index/v1/` and creates
or updates `AGENTS.md` in the target repository.

1. Build the release binary.

   ```bash
   cargo build -p rustrank --release
   ```

2. Confirm the registered MCP tools.

   ```bash
   target/release/rustrank --list-tools
   ```

   The command prints the tool names, including `index_project`, `context`,
   `impact`, `detect_changes`, and `query`.

3. Index a repository.

   ```bash
   target/release/rustrank index-project --repo-path . --clean-stale
   ```

   The command prints a JSON summary with the cache path, manifest path,
   indexed file count, per-language summaries, cache hits and misses, stale
   removals, and warnings.

4. Run the local quality gate before changing code.

   ```bash
   cargo fmt --all -- --check
   cargo check --workspace --all-targets --all-features
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace --all-features
   python3 -m py_compile scripts/smoke_http_json.py
   cargo run -p rustrank -- --list-tools
   ```

## What RustRank does

RustRank turns a repository into a compact analysis index that agents can query
without rereading every file. It keeps the cache inside the target repository so
the analysis travels with the working tree and can be refreshed after changes.

Core capabilities:

- Index supported source files into per-language JSON shards.
- Build a project manifest with modules, imports, graph nodes, graph edges, and
  lightweight process flows.
- Generate an `AGENTS.md` section that orients future agents to the indexed
  codebase.
- Search code with context and module ranking.
- Trace API usage, data identifiers, feature keywords, dependency impact, and
  execution paths.
- Report symbol context, impact, changed symbols, hotspots, and query-ranked
  graph matches.
- Run as a local stdio MCP server or as a stateless Streamable HTTP MCP server.

RustRank is designed for local repository analysis. It does not replace normal
source review, tests, or build tooling; it gives agents a high-signal map before
they inspect or edit code.

## Supported languages

RustRank detects source language from file extension, then applies repository
configuration and excludes. It indexes these languages:

| Language | Extensions |
| --- | --- |
| Python | `.py` |
| Rust | `.rs` |
| C# | `.cs` |
| TypeScript | `.ts`, `.tsx` |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |
| C | `.c`, `.h` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.c++`, `.hpp`, `.hh`, `.hxx`, `.h++` |
| Go | `.go` |

`.h` files default to C. If a repository stores C++ headers with `.h`
extensions, configure language overrides before indexing.

## Repository output

RustRank writes deterministic analysis artifacts under the repository you
index. Use a writable checkout for indexing and configuration changes.

```text
<repo>/.rustrank/index/v1/project_manifest.json
<repo>/.rustrank/index/v1/languages/<language>/index.json
<repo>/.rustrank/index/v1/languages/<language>/files/<blake3-hash>.json
<repo>/.rustrank/index/v1/embeddings/<blake3-hash>.json
```

The embedding directory appears only when embedding indexing is enabled.

The per-file cache stores metadata such as:

- relative source path
- language
- module name
- BLAKE3 content hash and size
- extracted symbols
- extracted imports
- declared C# namespaces

The cache intentionally does not store source snippets, full source lines,
absolute repository paths, timestamps, or secrets.

After indexing, RustRank creates or updates `AGENTS.md` in the target
repository. Manual content outside these markers is preserved:

```text
<!-- rustrank-index:start -->
<!-- rustrank-index:end -->
```

## CLI

Running `rustrank` without a utility flag starts the selected MCP transport.
Utility commands exit after printing their result.

```bash
rustrank --help
rustrank --list-tools
rustrank index-project --repo-path /path/to/repo
```

The `index-project` command accepts these options:

| Option | Purpose |
| --- | --- |
| `--repo-path <PATH>` | Repository path to index. Required. |
| `--languages <LANGUAGES>` | Comma-separated language names or aliases, such as `python,rust,typescript`. |
| `--force-rebuild` | Rebuild cache entries even when file hashes are unchanged. |
| `--clean-stale` | Remove stale cache files for source files that no longer exist. |
| `--embeddings` | Enable embedding generation for indexed files. |
| `--embedding-base-url <URL>` | Embedding API base URL. RustRank appends `/embeddings`. |
| `--embedding-model <MODEL>` | Embedding model name. |
| `--embedding-dims <N>` | Embedding vector dimensionality. |
| `--embedding-api-key <KEY>` | Embedding API key for the current run. |

Example index runs:

```bash
rustrank index-project --repo-path /path/to/repo --clean-stale
```

```bash
rustrank index-project \
  --repo-path /path/to/repo \
  --languages python,rust,typescript \
  --force-rebuild \
  --clean-stale
```

```bash
rustrank index-project \
  --repo-path /path/to/repo \
  --embeddings \
  --embedding-base-url https://api.phrk.org/v1 \
  --embedding-model text-image-embedding \
  --embedding-dims 1536
```

## MCP server

RustRank starts as a stdio MCP server by default. Use stdio for local MCP
clients that launch tools as child processes.

```bash
target/release/rustrank
```

For clients that accept a command path, build the binary and register its
absolute path:

```bash
cargo build -p rustrank --release
realpath target/release/rustrank
```

For Codex CLI stdio registration:

```bash
codex mcp add rustrank -- "$(realpath target/release/rustrank)"
```

## Streamable HTTP

RustRank can also run as a stateless Streamable HTTP MCP server. HTTP mode is
useful for Docker, remote clients, and smoke testing.

```bash
RUSTRANK_TRANSPORT=streamable_http \
RUSTRANK_HOST=127.0.0.1 \
RUSTRANK_PORT=63477 \
target/release/rustrank
```

The HTTP server exposes:

```text
POST /mcp
GET /healthz
```

Check local health:

```bash
curl -fsS http://127.0.0.1:63477/healthz
```

Register the HTTP endpoint with Codex CLI:

```bash
codex mcp add rustrank-http --url http://127.0.0.1:63477/mcp
```

### HTTP environment variables

These variables control HTTP transport. Legacy `RUSTANK_*` spellings are still
accepted for the RustRank-specific HTTP variables.

| Variable | Default | Purpose |
| --- | --- | --- |
| `RUSTRANK_TRANSPORT` | `stdio` | `http`, `streamable_http`, and `streamable-http` select HTTP. Any other value selects stdio. |
| `RUSTRANK_LISTEN_ADDR` | unset | Full socket address. Takes precedence over `RUSTRANK_HOST` and `RUSTRANK_PORT`. |
| `RUSTRANK_HOST` | `127.0.0.1` | Host used when `RUSTRANK_LISTEN_ADDR` is unset. Docker sets `0.0.0.0`. |
| `RUSTRANK_PORT` | `63477` | Port used when `RUSTRANK_LISTEN_ADDR` is unset. |
| `RUSTRANK_MCP_PATH` | `/mcp` | MCP path. Values are normalized with a leading slash; `/` and `/healthz` are rejected. |
| `RUSTRANK_ALLOWED_HOSTS` | loopback plus bound host | Comma-separated hostnames, IPs, or authorities accepted by host validation. |
| `RUSTRANK_ALLOWED_ORIGINS` | unset | Comma-separated origins. Empty means Origin validation is disabled. |
| `RUSTRANK_DISABLE_HOST_CHECK` | `false` | Set to `true`, `1`, `yes`, or `on` only on trusted networks. |
| `RUST_LOG` | unset locally | Standard Rust logging filter. The Docker image sets `info`. |

## Tool reference

RustRank registers 19 MCP tools. Use `rustrank --list-tools` to verify the
current list from the binary you are running.

| Tool | Use it to |
| --- | --- |
| `index_project` | Build persistent per-language caches, the project manifest, optional embedding caches, and the generated `AGENTS.md` section. |
| `contextual_search` | Search repository files for text or regex patterns with surrounding lines. |
| `smart_code_search` | Search code and rank matches by module importance. |
| `api_usage` | Find examples of an API, function, method, or identifier. |
| `coderank_analysis` | Rank modules with import-graph PageRank. |
| `code_hotspots` | Find connected modules with high rank and change or textual frequency. |
| `trace_data_flow` | Trace occurrences and simple transformations of a data identifier. |
| `trace_feature_impl` | Map feature keywords across source files and coarse layers. |
| `trace_dep_impact` | Find direct import dependents for a module. |
| `error_patterns` | Find error-handling patterns and optional antipatterns. |
| `perf_bottleneck` | Detect simple performance-pattern matches or custom focus strings. |
| `exec_paths` | Trace branches, loops, error paths, and optional call contexts in a function. |
| `execute_paths` | Alias for `exec_paths`. |
| `get_config` | Read `.rustrank_config.json`. |
| `set_config` | Set a top-level or dotted JSON configuration value. |
| `context` | Return callers, callees, imports, defining file, and resources for a symbol. |
| `impact` | Estimate upstream and downstream blast radius for a symbol or module. |
| `detect_changes` | Map git worktree diff hunks to changed symbols and affected callers or importers. |
| `query` | Run graph-aware search with lexical, centrality, process, and optional semantic signals. |

## MCP resources

MCP clients that support resources can read indexed repository context after
`index_project` runs.

| Resource | Contents |
| --- | --- |
| `rustrank://repo/current/context` | Repository summary with manifest, module count, node count, and graph edge count. |
| `rustrank://repo/current/schema` | Graph node and edge schema notes plus index freshness. |
| `rustrank://repo/current/modules` | Indexed module list with paths, symbols, and imports. |
| `rustrank://repo/current/processes` | Process flows inferred from call edges. |
| `rustrank://repo/current/module/{name}` | Symbols and imports for one indexed module. |
| `rustrank://repo/current/process/{name}` | Call chain for one inferred process flow. |

Resource reads use the current repository set by `index_project`. If no current
repository has been set, RustRank falls back to the server process working
directory.

## Configuration

RustRank stores repository configuration in `.rustrank_config.json` at the root
of the repository being analyzed. If the file is absent or has no enabled
languages, RustRank auto-detects supported source files.

Select languages explicitly:

```json
{
  "languages": {
    "enabled": ["python", "rust", "typescript"]
  }
}
```

Override ambiguous paths before extension mapping:

```json
{
  "languages": {
    "enabled": ["c", "cpp"],
    "overrides": [
      {
        "paths": ["include/cpp/**/*.h", "src/cxx/**/*.h"],
        "language": "cpp"
      }
    ]
  }
}
```

Extend default source excludes:

```json
{
  "excludes": {
    "paths": [".tox/**", "generated/**"],
    "extensions": ["sqlite", ".bin"]
  }
}
```

Configure embeddings for semantic `query` scoring:

```json
{
  "embeddings": {
    "enabled": true,
    "base_url": "https://api.phrk.org/v1",
    "model": "text-image-embedding",
    "dimensions": 1536
  }
}
```

Provide embedding API keys as request or CLI options for the current run. Do not
commit API keys to `.rustrank_config.json`.

## Indexing model

RustRank treats the persistent cache as a fast map, not as a replacement for the
repository. Tools that need source lines parse live files, and agent-facing
tools report stale-index warnings when the manifest head differs from the
current git head.

Indexing follows this flow:

1. Resolve enabled languages from request options or `.rustrank_config.json`.
2. Walk supported source files while applying default and configured excludes.
3. Compute a BLAKE3 content hash for each file.
4. Reuse compatible per-file cache entries when the content hash and cache
   header match.
5. Parse cache misses with RustPython or Tree-sitter.
6. Write per-language shard indexes.
7. Remove stale per-file caches when `clean_stale` is true.
8. Build `project_manifest.json` with modules, imports, graph nodes, graph
   edges, unresolved imports, process flows, and git freshness.
9. Update the generated `AGENTS.md` section and helper workflow docs under
   `.rustrank/skills/`.

## Docker

The Docker image runs RustRank in Streamable HTTP mode against repositories
mounted under `/workspace`.

Build the image:

```bash
docker build -t rustrank:local .
```

Run the server against the current repository:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v "$PWD:/workspace/repo" \
  rustrank:local
```

The image:

- runs `rustrank` as the entrypoint
- defaults to `RUSTRANK_TRANSPORT=streamable_http`
- listens on `0.0.0.0:63477`
- serves MCP at `/mcp`
- exposes `/healthz`
- runs as UID `10001`
- uses `/workspace` as the working directory

Use a read-write mount when calling `index_project` or `set_config`, because
those tools write `.rustrank_config.json`, `.rustrank/index/v1/`, and
`AGENTS.md`. A read-only mount is suitable only for read-only tools.

## HTTP smoke test

The smoke script verifies the no-SSE Streamable HTTP JSON path. It creates a
temporary multi-language fixture, initializes MCP, checks the tool list, indexes
the fixture, exercises resources, calls every expected tool, and verifies that
an embedding API key is not echoed in tool output.

Start a local server:

```bash
RUSTRANK_TRANSPORT=streamable_http \
RUSTRANK_HOST=127.0.0.1 \
RUSTRANK_PORT=63477 \
cargo run -p rustrank
```

In another shell, run the smoke test:

```bash
python3 scripts/smoke_http_json.py --url http://127.0.0.1:63477/mcp
```

For Docker smoke testing:

```bash
docker build -t rustrank:local .

fixture_dir="$(mktemp -d)"
chmod 0777 "$fixture_dir"

docker run -d --rm \
  --name rustrank-smoke \
  -p 127.0.0.1:63477:63477 \
  -v "$fixture_dir:/workspace/fixture" \
  rustrank:local

python3 scripts/smoke_http_json.py \
  --url http://127.0.0.1:63477/mcp \
  --fixture-dir "$fixture_dir" \
  --repo-path /workspace/fixture

docker stop rustrank-smoke
rm -rf "$fixture_dir"
```

## Development

RustRank is a single-crate Cargo workspace. The binary entrypoint delegates to
`rustrank::tools::serve()`, and most behavior lives in focused library modules.

| Area | Main files |
| --- | --- |
| Source discovery, parsing, imports, module resolution | `src/src/context.rs` |
| Raw and typed repository configuration | `src/src/project_config.rs` |
| Persistent cache, project manifest, generated `AGENTS.md` | `src/src/index.rs` |
| Optional embedding requests and cache handling | `src/src/embeddings.rs` |
| Process-flow derivation from call edges | `src/src/process.rs` |
| MCP router, CLI parsing, stdio and HTTP transports | `src/src/tools/mod.rs` |
| Agent resources, symbol context, impact, query, change detection | `src/src/tools/agent.rs` |
| Search, ranking, trace, analysis, and config handlers | `src/src/tools/*.rs` |
| Integration fixtures and behavior tests | `src/tests/` |

The project pre-push hook runs the main Rust checks:

```bash
.githooks/pre-push
```

Run the same checks manually when changing behavior:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run these additional checks when changing the CLI, documentation, or smoke test:

```bash
python3 -m py_compile scripts/smoke_http_json.py
cargo run -p rustrank -- --list-tools
cargo run -p rustrank -- --help
cargo run -p rustrank -- index-project --help
```

## Security and data handling

RustRank is intended to be safe for local agent use, but it still writes
analysis artifacts into the target repository. Review generated files before
committing them.

Important boundaries:

- The index cache stores metadata, not source snippets or full source lines.
- Cache paths are repository-relative, not absolute.
- `index_project` and `set_config` require write access to the target repo.
- Embedding API keys are accepted for the current request or CLI run and are
  redacted in debug output.
- Do not put API keys or bearer tokens in `.rustrank_config.json`.
- Host validation is enabled for HTTP mode unless
  `RUSTRANK_DISABLE_HOST_CHECK` is set.

## Troubleshooting

These are common first-run issues and the fastest checks to run.

| Symptom | Check |
| --- | --- |
| `No supported source found` | Confirm the repo has supported file extensions and that `.rustrank_config.json` does not exclude or filter them out. |
| `.h` files appear as C instead of C++ | Add `languages.overrides` rules for the relevant header paths. |
| `index-project` cannot write cache files | Use a writable checkout or mount the repo read-write in Docker. |
| HTTP clients receive host validation errors | Add the external hostname or authority to `RUSTRANK_ALLOWED_HOSTS`, or disable host checks only on a trusted network. |
| `query` does not use semantic scoring | Enable embeddings, provide a reachable embedding base URL, and use matching dimensions for cached embeddings and query embeddings. |
| Agent resources are missing | Call `index_project` first so the server has a current repository and manifest. |
| Impact or change reports mention a stale index | Re-run `index_project --clean-stale` after changing source files. |

## Documentation map

Use these files for deeper implementation and behavior details:

- [MCP server specification](docs/SPEC.md)
- [Implementation notes](docs/IMPLEMENTATION.md)
- [HTTP smoke script](scripts/smoke_http_json.py)
- [Dockerfile](Dockerfile)
- [Pre-push checks](.githooks/pre-push)

## Contributing

Keep changes small and source-backed. For behavior changes, add or update an
integration test in `src/tests/integration.rs`; for parser or fixture changes,
update the fixtures in `src/tests/fixtures.rs` or the HTTP smoke fixture as
needed.

Before opening a pull request or handing off changes, run the local quality
gate:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python3 -m py_compile scripts/smoke_http_json.py
cargo run -p rustrank -- --list-tools
```
