# RustRank

RustRank is an MCP server for repository analysis. It gives MCP clients tools
for indexing code, searching symbols and usages, ranking modules, tracing
dependencies, inspecting execution paths, reading or writing repository config,
and maintaining a generated RustRank section in `AGENTS.md`.

RustRank runs locally over stdio by default. It can also run as a stateless
Streamable HTTP server for Docker or remote-client setups.

## Quick start

Clone and build RustRank:

```bash
git clone https://git.phrk.org/pub/RustRank.git
cd RustRank
cargo build -p rustrank --release
```

List the registered MCP tools:

```bash
target/release/rustrank --list-tools
```

Run RustRank as a local stdio MCP server:

```bash
target/release/rustrank
```

Create or refresh the persistent index for a repository:

```bash
target/release/rustrank index-project --repo-path /path/to/repo
```

Index only selected languages and force a rebuild:

```bash
target/release/rustrank index-project \
  --repo-path /path/to/repo \
  --languages python,rust,typescript \
  --force-rebuild \
  --clean-stale
```

`index-project` prints JSON to stdout and writes repository-local files under
`<repo_path>/.rustrank/index/v1/`. It also creates or amends
`<repo_path>/AGENTS.md`.

## MCP tools

RustRank currently registers 15 MCP tools.

| Tool | Main arguments | What it does |
| --- | --- | --- |
| `index_project` | `repo_path`, optional `languages`, `force_rebuild`, `clean_stale` | Builds persistent per-language index shards, `project_manifest.json`, and the generated RustRank section in `AGENTS.md`. |
| `contextual_search` | `path`, `pattern`, optional `file_type`, `is_regex`, `num_context_lines` | Searches files under a directory and returns matching lines with before and after context. This is the raw filesystem search tool. |
| `smart_code_search` | `repo_path`, `pattern`, `context_lines`, `num_context_lines` | Searches supported source files and orders matches by CodeRank score. |
| `api_usage` | `repo_path`, `api_name`, `max_examples`, `group_by_pattern` | Finds usage examples for an API, function, method, or identifier and labels each example as a call, import, assignment, reference, or generic usage. |
| `coderank_analysis` | `repo_path`, `top_n`, optional `module_prefix`, `external_modules` | Ranks modules with import-graph PageRank. |
| `code_hotspots` | `repo_path`, `top_n`, `min_connections` | Finds modules with high graph connectivity and change or textual frequency. |
| `trace_data_flow` | `repo_path`, `identifier`, `include_transformations`, `include_side_effects` | Traces definitions, usages, assignments, returns, and raises for an identifier. |
| `trace_feature_impl` | `repo_path`, `feature_keywords` | Maps feature keywords across source files and coarse layers such as API, data, UI, tests, and business logic. |
| `trace_dep_impact` | `repo_path`, `target_module` | Finds direct import dependents of a target module. |
| `error_patterns` | `repo_path`, `include_antipatterns`, `show_evolution`, optional `days_back` | Finds error-handling patterns such as `try`, `except`, `raise`, `throw`, `panic!`, and `unwrap`. Can add git-blame evolution rows when available. |
| `perf_bottleneck` | `repo_path`, `focus_areas`, `include_utility` | Finds simple performance-pattern matches such as `sleep`, `range`, `append`, or `push`, or custom focus strings. |
| `exec_paths` | `repo_path`, `function_name`, `max_depth`, `include_call_contexts` | Traces branches, loops, error paths, and optionally calls inside a function. |
| `execute_paths` | Same as `exec_paths` | Alias for `exec_paths`. |
| `get_config` | `repo_path` | Reads raw RustRank repository configuration from `.rustrank_config.json`. |
| `set_config` | `repo_path`, `key`, `value` | Writes a top-level JSON config value to `.rustrank_config.json`. |

### Common MCP calls

Index a project through MCP:

```json
{
  "name": "index_project",
  "arguments": {
    "repo_path": "/path/to/repo",
    "languages": ["python", "rust", "typescript"],
    "force_rebuild": false,
    "clean_stale": true
  }
}
```

Configure enabled languages through MCP:

```json
{
  "name": "set_config",
  "arguments": {
    "repo_path": "/path/to/repo",
    "key": "languages",
    "value": {
      "enabled": ["python", "rust", "csharp", "typescript", "javascript"]
    }
  }
}
```

Read the current config:

```json
{
  "name": "get_config",
  "arguments": {
    "repo_path": "/path/to/repo"
  }
}
```

## Supported languages

RustRank supports five language families. The canonical names are the values to
use in `.rustrank_config.json`, `set_config`, and `index_project.languages`.

| Language | Canonical config name | Extensions | Accepted aliases | Extracted facts |
| --- | --- | --- | --- | --- |
| Python | `python` | `.py` | `py` | Imports, functions, classes, line ranges, and whether definitions have arguments. |
| Rust | `rust` | `.rs` | `rs` | `use` imports, functions, structs, enums, and traits. |
| C# | `csharp` | `.cs` | `c#`, `cs` | `using` imports, classes, records, structs, interfaces, enums, methods, constructors, and declared namespaces. |
| TypeScript | `typescript` | `.ts`, `.tsx` | `ts`, `tsx` | ES imports and exports, `require` calls, functions, classes, interfaces, enums, methods, and callable variable declarations. |
| JavaScript | `javascript` | `.js`, `.jsx`, `.mjs`, `.cjs` | `js`, `jsx`, `mjs`, `cjs` | ES imports and exports, `require` calls, functions, classes, methods, and callable variable declarations. |

RustRank skips source files under these ignored directories when building the
language-aware source set:

```text
.git
.rustrank
target
node_modules
dist
build
```

## Language detection and configuration

If no language config exists, or if `languages.enabled` is missing or empty,
RustRank auto-detects enabled languages by scanning supported source files under
the target repository.

Repository config lives at:

```text
<repo_path>/.rustrank_config.json
```

The language config shape is:

```json
{
  "languages": {
    "enabled": ["python", "rust", "csharp", "typescript", "javascript"]
  }
}
```

`set_config` writes one top-level key at a time, so set language config with
`key = "languages"` and a value containing `enabled`.

`index_project.languages` overrides repository config for that index run:

```json
{
  "repo_path": "/path/to/repo",
  "languages": ["py", "rs", "ts"],
  "force_rebuild": false,
  "clean_stale": true
}
```

Language configuration affects parser-backed analysis and indexing. That
includes `index_project`, `coderank_analysis`, `code_hotspots`,
`trace_data_flow`, `trace_feature_impl`, `trace_dep_impact`, `error_patterns`,
`perf_bottleneck`, `exec_paths`, and `execute_paths`.

`contextual_search` is intentionally broader: it searches files on disk and can
optionally filter by extension with `file_type`. `smart_code_search` and
`api_usage` search supported source extensions, then use parser-backed ranking
or pattern grouping where applicable.

Unsupported requested languages are reported as warnings by `index_project`.
If an explicit `languages` request contains no supported names, the request
fails with a validation error.

## Persistent indexing

`index_project` creates deterministic, repository-local JSON under:

```text
<repo_path>/.rustrank/index/v1/
```

The layout is:

```text
.rustrank/index/v1/
+-- project_manifest.json
+-- languages/
    +-- <language>/
        +-- index.json
        +-- files/
            +-- <blake3-content-hash>.json
```

Each supported language gets its own shard directory. For example:

```text
.rustrank/index/v1/languages/python/index.json
.rustrank/index/v1/languages/rust/index.json
.rustrank/index/v1/languages/csharp/index.json
.rustrank/index/v1/languages/typescript/index.json
.rustrank/index/v1/languages/javascript/index.json
```

### Per-file cache files

Each file cache contains:

- cache header: schema version, cache version, RustRank package version, and extractor version
- relative source path
- module name
- language
- BLAKE3 content hash and file size
- extracted symbols
- extracted imports
- C# declared namespaces

The cache intentionally does not store source lines, snippets, absolute
repository paths, timestamps, or secrets.

### Per-language shard files

Each `languages/<language>/index.json` file contains:

- cache header
- canonical language name
- entries for files indexed in that language
- relative path, module name, content hash, cache-file path, symbol count, and import count for each file

### Project manifest

`.rustrank/index/v1/project_manifest.json` is the aggregate project index. It
contains:

- cache header
- language shard references
- indexed modules with stable IDs such as `rust:src/lib.rs`
- resolved import edges between local modules
- unresolved imports for external packages or unresolved local imports

Use the project manifest when a client needs repository-level module, symbol,
and dependency context before making broad changes.

### Cache reuse, stale files, and rebuilds

RustRank hashes each source file with BLAKE3. A per-file cache is a hit when
all of these values match:

- cache header
- source content hash
- relative path

`force_rebuild = true` skips cache reuse and reparses all selected source
files. `clean_stale = true` removes old per-file JSON caches that no current
source file references. Language shard files and `project_manifest.json` are
rewritten on every successful index run.

The `index_project` response includes `scanned_files`, `indexed_files`,
`cache_hits`, `cache_misses`, `stale_removed`, per-language summaries, and
warnings.

### AGENTS.md generated section

After a successful index, RustRank creates `<repo_path>/AGENTS.md` if it does
not exist. If the file already exists, RustRank preserves manual content outside
the generated section.

The generated section is bounded by these markers:

```text
<!-- rustrank-index:start -->
<!-- rustrank-index:end -->
```

When both markers already exist, RustRank replaces only the content between
them. The generated section includes:

- `## RustRank Indexed Codebase`
- persistent index cache path
- project manifest path
- language shard summary table
- a privacy note about what the cache stores and excludes

Re-run `index_project` after source changes to refresh `.rustrank/index/v1/`
and the generated `AGENTS.md` section.

## Manual MCP installation

Build the release binary first:

```bash
cargo build -p rustrank --release
```

Use an absolute binary path in client configs:

```text
/home/mp/.mcp-servers/RustRank/target/release/rustrank
```

The MCP client process must be able to read any `repo_path` you pass to tools.
Use a writable repository path when calling `set_config` or `index_project`,
because those tools write `.rustrank_config.json`, `.rustrank/index/v1/`, and
possibly `AGENTS.md`.

### Claude Code

For local stdio, add this to your MCP JSON configuration:

```json
{
  "mcpServers": {
    "rustrank": {
      "command": "/home/mp/.mcp-servers/RustRank/target/release/rustrank",
      "args": []
    }
  }
}
```

For HTTP, start RustRank with `RUSTRANK_TRANSPORT=streamable_http` or use the
Docker quickstart, then add:

```json
{
  "mcpServers": {
    "rustrank": {
      "type": "http",
      "url": "http://127.0.0.1:63477/mcp"
    }
  }
}
```

### OpenCode

For local stdio, add this to `opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "rustrank": {
      "type": "local",
      "command": ["/home/mp/.mcp-servers/RustRank/target/release/rustrank"],
      "enabled": true
    }
  }
}
```

For HTTP:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "rustrank": {
      "type": "remote",
      "url": "http://127.0.0.1:63477/mcp",
      "enabled": true
    }
  }
}
```

### Codex CLI

Codex CLI stores MCP servers in `~/.codex/config.toml`. Add RustRank with the
CLI:

```bash
codex mcp add rustrank -- /home/mp/.mcp-servers/RustRank/target/release/rustrank
```

The generated TOML entry is:

```toml
[mcp_servers.rustrank]
command = "/home/mp/.mcp-servers/RustRank/target/release/rustrank"
```

The equivalent JSON view from `codex mcp get --json rustrank` is:

```json
{
  "name": "rustrank",
  "enabled": true,
  "disabled_reason": null,
  "transport": {
    "type": "stdio",
    "command": "/home/mp/.mcp-servers/RustRank/target/release/rustrank",
    "args": [],
    "env": null,
    "env_vars": [],
    "cwd": null
  },
  "enabled_tools": null,
  "disabled_tools": null,
  "startup_timeout_sec": null,
  "tool_timeout_sec": null
}
```

For HTTP:

```bash
codex mcp add rustrank-http --url http://127.0.0.1:63477/mcp
```

The generated TOML entry is:

```toml
[mcp_servers.rustrank-http]
url = "http://127.0.0.1:63477/mcp"
```

The equivalent JSON view is:

```json
{
  "name": "rustrank-http",
  "enabled": true,
  "disabled_reason": null,
  "transport": {
    "type": "streamable_http",
    "url": "http://127.0.0.1:63477/mcp",
    "bearer_token_env_var": null,
    "http_headers": null,
    "env_http_headers": null
  },
  "enabled_tools": null,
  "disabled_tools": null,
  "startup_timeout_sec": null,
  "tool_timeout_sec": null
}
```

### Cursor

For local stdio, add this to Cursor's MCP JSON configuration:

```json
{
  "mcpServers": {
    "rustrank": {
      "command": "/home/mp/.mcp-servers/RustRank/target/release/rustrank",
      "args": []
    }
  }
}
```

For HTTP:

```json
{
  "mcpServers": {
    "rustrank": {
      "type": "http",
      "url": "http://127.0.0.1:63477/mcp"
    }
  }
}
```

## Docker quickstart

The Docker image runs RustRank as Streamable HTTP by default. It listens on
`0.0.0.0:63477` inside the container and serves MCP at `/mcp`.

Build the image:

```bash
docker build -t rustrank:local .
```

Run RustRank against the current repository:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v "$PWD:/workspace/repo" \
  rustrank:local
```

Use this MCP URL from clients on the same host:

```text
http://127.0.0.1:63477/mcp
```

Check container health:

```bash
curl -fsS http://127.0.0.1:63477/healthz
```

Mount the target repository read-write when you call `set_config` or
`index_project`:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v /host/path/to/repo:/workspace/repo \
  rustrank:local
```

Use a read-only mount only for analysis tools that do not write config,
indexes, or `AGENTS.md`:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v /host/path/to/repo:/workspace/repo:ro \
  rustrank:local
```

The runtime user is UID `10001` (`rustrank`). For `set_config` and
`index_project`, the mounted repository must be writable by that UID. Those
tools may write:

```text
/workspace/repo/.rustrank_config.json
/workspace/repo/.rustrank/index/v1/
/workspace/repo/AGENTS.md
```

For remote clients, publish the port and allow the host header that reaches the
container:

```bash
docker run --rm \
  --name rustrank \
  -p 63477:63477 \
  -e RUSTRANK_ALLOWED_HOSTS="rustrank.example.com,rustrank.example.com:63477" \
  -v /srv/repos/my-repo:/workspace/my-repo \
  rustrank:local
```

## Transports

### Stdio

Stdio is the default transport:

```bash
target/release/rustrank
```

This is the simplest local setup for MCP clients that launch servers as child
processes.

### Streamable HTTP

Run the HTTP server locally:

```bash
RUSTRANK_TRANSPORT=streamable_http \
RUSTRANK_HOST=127.0.0.1 \
RUSTRANK_PORT=63477 \
target/release/rustrank
```

The MCP endpoint is:

```text
POST http://127.0.0.1:63477/mcp
```

The health endpoint is:

```text
GET http://127.0.0.1:63477/healthz
```

RustRank's HTTP mode is stateless Streamable HTTP with JSON responses. It does
not create HTTP sessions or SSE streams. MCP HTTP clients should send
`Content-Type: application/json`, `Accept: application/json, text/event-stream`,
and an MCP protocol version header supported by the client and RMCP server.

## Environment variables and options

Canonical environment variables use the `RUSTRANK_` prefix. RustRank also
accepts the original misspelled `RUSTANK_` aliases listed below for
compatibility.

| Variable | Default | Supported values or format | Notes |
| --- | --- | --- | --- |
| `RUSTRANK_TRANSPORT` | `stdio` for the local binary; `streamable_http` in Docker | `stdio`, `http`, `streamable_http`, `streamable-http` | `http`, `streamable_http`, and `streamable-http` select HTTP. Unset, `stdio`, or any other value selects stdio. |
| `RUSTRANK_LISTEN_ADDR` | unset | Socket address such as `127.0.0.1:63477` or `0.0.0.0:9000` | Takes precedence over `RUSTRANK_HOST` and `RUSTRANK_PORT` in HTTP mode. |
| `RUSTRANK_HOST` | `127.0.0.1` for the local binary; `0.0.0.0` in Docker | IP address or host accepted by Rust socket parsing | Used with `RUSTRANK_PORT` when `RUSTRANK_LISTEN_ADDR` is unset. |
| `RUSTRANK_PORT` | `63477` | TCP port number | Used with `RUSTRANK_HOST` when `RUSTRANK_LISTEN_ADDR` is unset. |
| `RUSTRANK_MCP_PATH` | `/mcp` | HTTP path | Values without a leading slash are normalized. Trailing slashes are removed. `/` and `/healthz` are rejected. |
| `RUSTRANK_ALLOWED_HOSTS` | `localhost`, `127.0.0.1`, `::1`, the bound host, and `bound-host:port` | Comma-separated hosts, IPs, or `host:port` authorities | Add public DNS names, reverse-proxy hostnames, or published `host:port` values for HTTP clients. |
| `RUSTRANK_ALLOWED_ORIGINS` | unset | Comma-separated origins such as `https://app.example.com` | Empty means Origin validation is disabled. Missing `Origin` headers are accepted. |
| `RUSTRANK_DISABLE_HOST_CHECK` | `false` | `true`, `1`, `yes`, or `on` to enable; anything else is false | Disables RMCP Host validation. Use only on trusted networks. |
| `RUST_LOG` | unset for the local binary; `info` in Docker | Standard Rust logging filter | Set by the Docker image for compatibility with logging-aware dependencies. RustRank's HTTP startup line is written to stderr. |

Compatibility aliases:

| Canonical variable | Legacy alias |
| --- | --- |
| `RUSTRANK_TRANSPORT` | `RUSTANK_TRANSPORT` |
| `RUSTRANK_LISTEN_ADDR` | `RUSTANK_LISTEN_ADDR` |
| `RUSTRANK_HOST` | `RUSTANK_HOST` |
| `RUSTRANK_PORT` | `RUSTANK_PORT` |
| `RUSTRANK_MCP_PATH` | `RUSTANK_MCP_PATH` |
| `RUSTRANK_ALLOWED_HOSTS` | `RUSTANK_ALLOWED_HOSTS` |
| `RUSTRANK_ALLOWED_ORIGINS` | `RUSTANK_ALLOWED_ORIGINS` |
| `RUSTRANK_DISABLE_HOST_CHECK` | `RUSTANK_DISABLE_HOST_CHECK` |

## Smoke test

Start an HTTP server, then run:

```bash
scripts/smoke_http_json.py --url http://127.0.0.1:63477/mcp
```

The smoke test creates a temporary multi-language fixture, initializes the MCP
endpoint, verifies `tools/list`, rejects SSE responses, calls `index_project`,
checks generated `AGENTS.md`, and calls every registered RustRank MCP tool.

To smoke-test Docker:

```bash
docker build -t rustrank:local .

fixture_dir="$(mktemp -d)"
chmod 0777 "$fixture_dir"

docker run -d --rm \
  --name rustrank-smoke \
  -p 127.0.0.1:63477:63477 \
  -v "$fixture_dir:/workspace/fixture" \
  rustrank:local

scripts/smoke_http_json.py \
  --url http://127.0.0.1:63477/mcp \
  --fixture-dir "$fixture_dir" \
  --repo-path /workspace/fixture

docker stop rustrank-smoke
rm -rf "$fixture_dir"
```

## Developing

Common commands:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python3 -m py_compile scripts/smoke_http_json.py
cargo run -p rustrank -- --list-tools
```

Run the CLI indexer against a local fixture or repository:

```bash
cargo run -p rustrank -- index-project --repo-path /path/to/repo
```

Run the local HTTP server:

```bash
RUSTRANK_TRANSPORT=streamable_http cargo run -p rustrank
```

Install the repository pre-push hook:

```bash
git config core.hooksPath .githooks
```

The pre-push hook currently runs:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

For HTTP or Docker changes, also run the smoke test above.
