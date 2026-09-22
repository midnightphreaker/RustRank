# RustRank

Repository: https://git.phrk.org/pub/RustRank

RustRank is a Rust MCP server for repository analysis. It indexes source files into a repository-local cache, exposes MCP tools for code search and graph-oriented inspection, and writes an `AGENTS.md` section that summarizes the indexed codebase for future agent work.

RustRank runs as a local stdio MCP server by default. It can also run as a stateless Streamable HTTP server for Docker, remote clients, or smoke testing.

## Quick Start

Build the binary:

```bash
cargo build -p rustrank --release
```

List the registered MCP tools:

```bash
target/release/rustrank --list-tools
```

Index a repository:

```bash
target/release/rustrank index-project --repo-path /path/to/repo --clean-stale
```

Run RustRank as a stdio MCP server:

```bash
target/release/rustrank
```

For development, the same paths work through Cargo:

```bash
cargo run -p rustrank -- --list-tools
cargo run -p rustrank -- index-project --repo-path /path/to/repo --clean-stale
```

## What RustRank Writes

`index_project` and the `index-project` CLI command write deterministic JSON under the target repository:

```text
repo_path/.rustrank/index/v1/
```

The index contains:

- `project_manifest.json`: repository-level modules, graph nodes, import edges, unresolved imports, process flows, and freshness metadata.
- `languages/LANGUAGE/index.json`: one shard per indexed language.
- `languages/LANGUAGE/files/CONTENT_HASH.json`: per-file facts keyed by source content hash.
- `embeddings/CHUNK_ID.json`: optional function/source chunk vectors with source line spans, model and source freshness metadata.

RustRank also creates or updates:

```text
repo_path/AGENTS.md
```

Manual AGENTS content is preserved outside the generated section:

```text
<!-- rustrank-index:start -->
<!-- rustrank-index:end -->
```

The cache stores metadata such as relative source paths, module names, symbols, imports, declared C# namespaces, graph edges, and file hashes. It is designed not to store source snippets, absolute repository paths, timestamps, or secrets.

Use a writable `repo_path` for `index_project` and `set_config`; those operations can write `.rustrank_config.json`, `.rustrank/index/v1/`, and `AGENTS.md`.

## Supported Languages

RustRank detects supported languages from source extensions, then applies repository configuration and excludes.

| Language | Config name | Extensions | Accepted aliases |
| --- | --- | --- | --- |
| Python | `python` | `.py` | `py` |
| Rust | `rust` | `.rs` | `rs` |
| C# | `csharp` | `.cs` | `c#`, `cs` |
| TypeScript | `typescript` | `.ts`, `.tsx` | `ts`, `tsx` |
| JavaScript | `javascript` | `.js`, `.jsx`, `.mjs`, `.cjs` | `js`, `jsx`, `mjs`, `cjs` |
| C | `c` | `.c`, `.h` | `c` |
| C++ | `cpp` | `.cpp`, `.cc`, `.cxx`, `.c++`, `.hpp`, `.hh`, `.hxx`, `.h++` | `c++`, `cc`, `cxx`, `h++`, `hh`, `hpp`, `hxx` |
| Go | `go` | `.go` | `golang` |

`.h` files are classified as C by default. Use language overrides when a repository stores C++ headers with `.h` extensions.

Default source excludes include `.git`, `.rustrank`, `target`, `node_modules`, `dist`, `build`, Python virtual environments, Python bytecode/cache directories, common binary media extensions, archives, and object/library outputs.

## Configuration

Repository configuration lives at:

```text
repo_path/.rustrank_config.json
```

When `languages.enabled` is missing or empty, RustRank auto-detects languages from current source files:

```json
{
  "languages": {
    "enabled": ["python", "rust", "typescript"]
  }
}
```

Path overrides are checked before extension mapping:

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

Additional excludes can be configured with path globs and extensions:

```json
{
  "excludes": {
    "paths": [".tox/**", "generated/**"],
    "extensions": ["sqlite", ".bin"]
  }
}
```

### MCP embedding endpoint

Configure the MCP server process before starting RustRank:

```bash
export RUSTRANK_EMBEDDING_BASE_URL=https://api.example.com/v1
export RUSTRANK_EMBEDDING_MODEL=your-embedding-model
export RUSTRANK_EMBEDDING_DIMS=1536
# Optional: set RUSTRANK_EMBEDDING_API_KEY in the server environment if required.
```

The base URL, model and positive integer dimensions enable embeddings for MCP
indexing and semantic `query`. Structural indexing works without them. The key is optional; unset or blank means no
Authorization header. These four settings are no longer tool arguments and
repository configuration cannot override them for MCP indexing. Pass the
variables to the server process in your MCP client's environment settings, or
with Docker `-e` options.

At startup RustRank sends a small test input to `<base_url>/embeddings`, with a
five-second request timeout. It checks HTTP success, the embedding response and
vector dimensions. Missing/invalid settings, connection or authentication
failures, unsupported models/endpoints, and malformed or mismatched vectors are
logged to stderr as a debug diagnostic. RustRank continues serving every tool.
`index_project` still builds the structural index and includes the saved problem
in its warnings; no embedding requests are made. `query` still performs text
and graph matching. Correct the server environment or endpoint and restart
RustRank to enable embeddings. HTTP client connections reuse the startup result
rather than probing again.

After successful validation, `index_project` generates embeddings by default;
`embeddings: false` skips vector generation for that call. MCP `query` uses the
same endpoint, model, dimensions and optional API key as indexing, including
when the repository has no embedding configuration.

Source is split at function boundaries; oversized functions and module-level
text are split further into chunks of at most 4,096 UTF-8 bytes and 80 lines.
These are byte/line limits, not a model-specific token guarantee. Chunk vectors
retain file, symbol and line locations, so semantic scores apply to the matching
chunk rather than every function in a file. Cache identity includes source and
chunk contents, location, endpoint, model and dimensions. Query ignores vectors
from changed/deleted files, other endpoints/models and the legacy whole-file
format. Re-run indexing to generate chunk vectors after upgrading; old cache
files can remain on disk but do not affect results.

Later embedding request failures are returned as indexing warnings; semantic
query falls back to text and graph matching. The standalone `index-project` CLI
and library callers retain their existing optional repository configuration and
embedding flags; MCP operations use the server environment.

## CLI

General help:

```bash
cargo run -p rustrank -- --help
```

Index command help:

```bash
cargo run -p rustrank -- index-project --help
```

Index a repository with selected languages and a clean rebuild:

```bash
cargo run -p rustrank -- index-project \
  --repo-path /path/to/repo \
  --languages python,rust,typescript \
  --force-rebuild \
  --clean-stale
```

Enable embedding generation for an index run:

```bash
cargo run -p rustrank -- index-project \
  --repo-path /path/to/repo \
  --embeddings \
  --embedding-base-url https://api.example.com/v1 \
  --embedding-model text-image-embedding \
  --embedding-dims 1536
```

`index-project` prints a JSON summary with cache paths, scanned and indexed file counts, cache hits and misses, stale cache removals, per-language summaries, and warnings.

## MCP Tools

`rustrank --list-tools` currently prints 19 MCP tools:

| Tool | Purpose |
| --- | --- |
| `index_project` | Build persistent per-language caches, the project manifest, optional embedding cache files, and the generated AGENTS section. |
| `contextual_search` | Search repository files for a text or regex pattern with line context. |
| `smart_code_search` | Search supported source files and rank results by module importance. |
| `api_usage` | Find examples of an API, function, method, or identifier. |
| `coderank_analysis` | Rank modules with import-graph PageRank. |
| `code_hotspots` | Find modules with high connectivity and textual/change frequency. |
| `trace_data_flow` | Trace definitions, usages, assignments, returns, and raises for an identifier. |
| `trace_feature_impl` | Map feature keywords across source files and coarse code layers. |
| `trace_dep_impact` | Find direct import dependents of a target module. |
| `error_patterns` | Find error-handling patterns and optional antipatterns. |
| `perf_bottleneck` | Detect simple performance-pattern matches or custom focus strings. |
| `exec_paths` | Trace branches, loops, and optional call contexts inside a function. |
| `execute_paths` | Alias for `exec_paths`. |
| `get_config` | Read raw RustRank JSON configuration. |
| `set_config` | Write a top-level or dotted JSON configuration value. |
| `context` | Return callers, callees, imports, defining file, and related resources for a symbol. |
| `impact` | Estimate upstream and downstream blast radius for a symbol or module. |
| `detect_changes` | Map git worktree diff hunks to changed symbols and affected callers/importers. |
| `query` | Run agent-oriented graph search with lexical, centrality, process, and optional semantic signals. |

The MCP server also exposes resources for clients that support MCP resources:

```text
rustrank://repo/current/context
rustrank://repo/current/schema
rustrank://repo/current/modules
rustrank://repo/current/processes
rustrank://repo/current/module/{name}
rustrank://repo/current/process/{name}
```

Resource reads use the current repository set by `index_project` or fall back to the server process working directory.

## Stdio

Stdio is RustRank's native default: it reads MCP messages from standard input,
writes responses to standard output, and opens no network port. Build the release
binary and use its absolute path:

```bash
cargo build -p rustrank --release
realpath target/release/rustrank
```

Use the path printed by `realpath` as the MCP command value in clients that accept JSON server configuration.

Codex CLI stdio registration:

```bash
codex mcp add rustrank -- "$(realpath target/release/rustrank)"
```

The Docker image defaults to HTTP, so explicitly select stdio and keep standard
input open when using it as an MCP server:

```bash
docker run --rm -i \
  -e RUSTRANK_TRANSPORT=stdio \
  -v "$PWD:/workspace/repo" \
  rustrank:local
```

`RUSTRANK_TRANSPORT` is unset (or `stdio`) for stdio. The aliases `http`,
`streamable_http`, and `streamable-http` select Streamable HTTP instead.

## MCP Client Setup

For HTTP clients, start RustRank with Streamable HTTP and register the URL:

```bash
RUSTRANK_TRANSPORT=streamable_http \
RUSTRANK_HOST=127.0.0.1 \
RUSTRANK_PORT=63477 \
target/release/rustrank
```

```text
http://127.0.0.1:63477/mcp
```

Codex CLI HTTP registration:

```bash
codex mcp add rustrank-http --url http://127.0.0.1:63477/mcp
```

## Streamable HTTP

The default transport is stdio. HTTP mode is selected with:

```bash
RUSTRANK_TRANSPORT=streamable_http target/release/rustrank
```

The HTTP server exposes:

```text
POST /mcp
GET /healthz
```

Local health check:

```bash
curl -fsS http://127.0.0.1:63477/healthz
```

Environment variables:

| Variable | Default | Notes |
| --- | --- | --- |
| `RUSTRANK_TRANSPORT` | stdio | `http`, `streamable_http`, and `streamable-http` select HTTP. Unset, `stdio`, or other values select stdio. |
| `RUSTRANK_LISTEN_ADDR` | unset | Full socket address. Takes precedence over `RUSTRANK_HOST` and `RUSTRANK_PORT`. |
| `RUSTRANK_HOST` | `127.0.0.1` | Host used when `RUSTRANK_LISTEN_ADDR` is unset. Docker sets `0.0.0.0`. |
| `RUSTRANK_PORT` | `63477` | Port used when `RUSTRANK_LISTEN_ADDR` is unset. |
| `RUSTRANK_MCP_PATH` | `/mcp` | HTTP MCP path. Values are normalized with a leading slash; `/` and `/healthz` are rejected. |
| `RUSTRANK_ALLOWED_HOSTS` | loopback hosts plus bound host/host:port | Comma-separated hostnames, IPs, or authorities accepted by RMCP host validation. |
| `RUSTRANK_ALLOWED_ORIGINS` | unset | Comma-separated origins. Empty means Origin validation is disabled. |
| `RUSTRANK_DISABLE_HOST_CHECK` | `false` | `true`, `1`, `yes`, and `on` disable allowed-host checks. Use only on trusted networks. |
| `RUST_LOG` | unset locally, `info` in Docker | Standard Rust logging filter. |

Legacy `RUSTANK_*` spellings are still read for the RustRank-specific HTTP variables.

## Docker

Build the image:

```bash
docker build -t rustrank:local .
```

Run the HTTP server against a mounted repository:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v "$PWD:/workspace/repo" \
  rustrank:local
```

The Docker image:

- runs `rustrank` as the entrypoint
- defaults to Streamable HTTP on `0.0.0.0:63477`
- serves MCP at `/mcp`
- exposes `/healthz`
- runs as UID `10001`
- uses `/workspace` as the working directory

Use a read-write mount when calling `index_project` or `set_config`, because those tools write to the target repository. A read-only mount is suitable only for tools that do not write config, indexes, or `AGENTS.md`.

For remote HTTP clients, add the externally visible hostname or `host:port` to `RUSTRANK_ALLOWED_HOSTS`.

## Development

The repository is a Cargo workspace with one crate in `src/`. The binary entrypoint delegates to `rustrank::tools::serve()`, which handles CLI utility paths, stdio transport, and Streamable HTTP transport.

Core modules:

| Module | Responsibility |
| --- | --- |
| `context` | Source discovery, language detection, parsing, module definitions, and import resolution. |
| `project_config` | Raw JSON config I/O, language selection, path overrides, and excludes. |
| `index` | Persistent cache generation, manifest generation, embedding indexing integration, and AGENTS section updates. |
| `embeddings` | OpenAI-compatible embedding requests, cache reads/writes, and semantic scoring. |
| `process` | Lightweight process-flow derivation from call edges. |
| `tools::*` | MCP request handlers, resources, CLI routing, transports, and JSON formatting. |
| `fmt` | Shared output row types. |
| `error` | Application error and crate result types. |

Common local commands:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python3 -m py_compile scripts/smoke_http_json.py
cargo run -p rustrank -- --list-tools
```

The repository includes a pre-push hook script with the main Rust checks:

```bash
.githooks/pre-push
```

## Smoke Testing HTTP

Start a local HTTP server:

```bash
RUSTRANK_TRANSPORT=streamable_http \
RUSTRANK_HOST=127.0.0.1 \
RUSTRANK_PORT=63477 \
cargo run -p rustrank
```

In another shell, run the no-SSE Streamable HTTP smoke test:

```bash
python3 scripts/smoke_http_json.py --url http://127.0.0.1:63477/mcp
```

To exercise vector generation, start the server with the valid embedding environment above; without it the smoke test exercises structural fallback. It creates a temporary multi-language fixture, initializes MCP, verifies the tool list, calls `index_project` with and without vector generation, exercises resources, and calls each expected tool. Startup failures and optional authentication are covered by the Rust MCP compatibility tests.

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

## Release Signals

The Forgejo release workflow runs automatically when a push to the default branch changes the application version. Manual runs offer `Major` (increment the middle component), `minor` (increment the final component), and `retry` (publish the current version). It installs Rust 1.98.1 plus `clippy` and `rustfmt`, then runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo build --release --locked -p rustrank
```

After checks and builds pass, the workflow commits synchronized version files, packages the Linux amd64 binary as `RustRank.vVERSION-linux-amd64.tar.gz`, and publishes the release at https://git.phrk.org/pub/RustRank/releases. Publication retries reuse the same version. The workflow requires `RELEASE_TOKEN` and `RELEASE_USER` secrets with repository and release write access.

## Additional Validation

`README_REAL_REPO_VALIDATION.md` documents optional validation against pinned external C, C++, and Go repositories. Use it after parser, indexing, search, or graph-ranking changes when fixture tests are not enough.

Implementation details are summarized in `docs/IMPLEMENTATION.md`; the protocol and behavior spec lives in `docs/SPEC.md`.
