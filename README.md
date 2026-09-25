# RustRank

<p align="center"><img src="src/src/assets/rustrank.png" width="128" height="128" alt="RustRank crab and magnifying glass"></p>

RustRank helps coding assistants find relevant code, understand dependencies, and assess the impact of a change. It combines source parsing, import-graph ranking, and optional semantic search in a Rust [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server, with a standalone indexing CLI.

- **Eight languages:** Python, Rust, C#, TypeScript, JavaScript, C, C++, and Go.
- **19 MCP tools:** indexing, code search, symbol context, dependency inspection, and change analysis.
- **Optional embeddings:** search function-aware source chunks using an OpenAI-compatible embedding endpoint. Structural indexing and text/graph search work without one.
- **Local indexes:** cache source facts in the repository and generate guidance for future agent sessions.
- **Two transports:** local stdio or Streamable HTTP with JSON responses.

[Project on GitHub](https://github.com/midnightphreaker/RustRank) · [Forgejo source](https://git.phrk.org/pub/RustRank) · [Releases](https://git.phrk.org/pub/RustRank/releases)

## Contents

- [Quickstart](#quickstart)
- [Connect an MCP client](#connect-an-mcp-client)
- [Typical workflow](#typical-workflow)
- [Configuration](#configuration)
- [Standalone CLI](#standalone-cli)
- [Docker setup](#docker-setup)
- [Tools and resources](#tools-and-resources)
- [Supported languages](#supported-languages)
- [Files RustRank writes](#files-rustrank-writes)
- [Definitions](#definitions)
- [Troubleshooting and limits](#troubleshooting-and-limits)
- [Development](#development)

## Quickstart

You need Git, a Rust toolchain with Cargo, and the native compiler/linker tools required to build Rust dependencies. The project uses Rust edition 2024; CI currently tests Rust **1.98.1**. An embedding service is optional.

Build from source:

```bash
git clone https://git.phrk.org/pub/RustRank
cd RustRank
cargo build --locked --release -p rustrank
```

Check the binary and index RustRank itself as a first run:

```bash
./target/release/rustrank --list-tools
./target/release/rustrank index-project --repo-path .
# Prints a JSON summary; writes .rustrank/index/v1/project_manifest.json.
```

Replace `.` with the path to your own repository when ready. The CLI summary includes indexed file counts, language summaries, cache hits/misses, and warnings.

For a prebuilt binary, see [Releases](https://git.phrk.org/pub/RustRank/releases). The release workflow currently targets **Linux amd64**. The instructions below assume a source build; substitute your installed binary path if using a release archive.

## Connect an MCP client

### Local stdio

Configure your client to launch the binary using an **absolute path**. In clients that accept an `mcpServers` JSON configuration:

```json
{
  "mcpServers": {
    "rustrank": {
      "command": "/absolute/path/to/RustRank/target/release/rustrank",
      "args": [],
      "env": {
        "RUSTRANK_TRANSPORT": "stdio"
      }
    }
  }
}
```

Adapt the configuration wrapper to your client. No server arguments are needed. Stdio is the native default: the client communicates through stdin/stdout, and RustRank opens no listening port. Diagnostics go to stderr.

Launching the binary directly in a terminal starts the MCP server and waits for protocol messages; it is not an interactive command prompt.

### Streamable HTTP

Start a local HTTP server:

```bash
RUSTRANK_TRANSPORT=streamable_http \
RUSTRANK_HOST=127.0.0.1 \
RUSTRANK_PORT=63477 \
./target/release/rustrank
```

Register `http://127.0.0.1:63477/mcp` as a **Streamable HTTP** server in your client. Check service availability from another terminal:

```bash
curl -fsS http://127.0.0.1:63477/healthz
# ok
```

HTTP uses stateless JSON responses, without an SSE event stream. `/healthz` confirms the HTTP service is running; it does not validate the embedding endpoint.

For a reverse proxy or remote hostname, configure `RUSTRANK_ALLOWED_HOSTS` with the hostname or `host:port` clients use. Host/origin checks are not user authentication; remote deployments need an appropriate access-control layer.

## Typical workflow

1. **Index:** call `index_project` before exploration and after source changes.
2. **Find:** use `query` for ranked module/symbol/chunk matches, or `contextual_search` for exact text and regex searches.
3. **Inspect:** use `context` for a symbol and `impact` before changing shared code.
4. **Review:** use `detect_changes` for unstaged tracked-file edits, then refresh the index.

Example arguments for the MCP **`index_project`** tool:

```json
{
  "repo_path": "/absolute/path/to/repo",
  "force_rebuild": false,
  "clean_stale": false
}
```

`repo_path`, `force_rebuild`, and `clean_stale` are required. Optional `languages` selects a list of languages; optional `embeddings` defaults to enabled when the server has a validated endpoint. Set it to `false` to skip vector generation for that run. Endpoint details and API keys are **server configuration, not tool arguments**.

Example arguments for **`query`**:

```json
{
  "repo_path": "/absolute/path/to/repo",
  "query": "authentication token validation",
  "limit": 10
}
```

Paths are resolved on the machine running RustRank. A remote server cannot read a client-local path, and Docker tool calls must use the mounted container path.

## Configuration

There are two configuration scopes:

| Scope | Location | Controls |
| --- | --- | --- |
| MCP server | Process environment / MCP client's server environment | Transport, HTTP settings, embedding endpoint and optional API key. |
| Repository | `<repo_path>/.rustrank_config.json` | Enabled languages, path overrides, exclusions; optional embedding settings for CLI/library callers. |

**MCP indexing and semantic query share the same environment-based embedding settings.** Repository embedding settings cannot override them. The standalone CLI uses its own flags and repository settings; see [Standalone CLI](#standalone-cli).

### Repository settings

No config file is required. To customize a repository, create `.rustrank_config.json` in its root:

```json
{
  "languages": {
    "enabled": ["python", "rust", "cpp"],
    "overrides": [
      { "paths": ["include/**/*.h"], "language": "cpp" }
    ]
  },
  "excludes": {
    "paths": ["generated/**", "vendor/**"],
    "extensions": ["sqlite", ".bin"]
  }
}
```

- Missing or empty `languages.enabled` means auto-detect supported languages. Invalid names are reported during indexing; if none are valid, detection is used.
- Path overrides are checked before extension mapping; the first matching rule wins. Use them for C++ headers named `.h`, which otherwise default to C.
- Custom excludes extend the defaults. Root-level `.git`, `.rustrank`, `target`, `node_modules`, `dist`, `build`, `.venv`, `venv`, and `.pytest_cache` are excluded, along with `__pycache__` directories and common binary/archive extensions.
- MCP `get_config` reads this file. `set_config` writes a JSON value at a dotted key, such as `languages.enabled`. It does not rebuild the index.

### Optional embedding endpoint

Set these variables in the **RustRank server process** before starting it:

| Variable | Requirement | Meaning |
| --- | --- | --- |
| `RUSTRANK_EMBEDDING_BASE_URL` | Required for embeddings | HTTP(S) API base, for example `https://api.example.com/v1`. RustRank appends `/embeddings`. |
| `RUSTRANK_EMBEDDING_MODEL` | Required for embeddings | Model identifier accepted by that endpoint. |
| `RUSTRANK_EMBEDDING_DIMS` | Required for embeddings | Positive integer matching the returned vector dimensions. |
| `RUSTRANK_EMBEDDING_API_KEY` | Optional | Bearer token. Unset or blank sends no Authorization header. |
| `RUSTRANK_EMBEDDING_MAX_INPUT_TOKENS` | Optional | Requested input ceiling. RustRank reads the selected model's `max_input_tokens` from `/model/info` or `/models` when available and uses the smaller value if both are present. |

For a shell-launched server:

```bash
export RUSTRANK_EMBEDDING_BASE_URL=https://api.example.com/v1
export RUSTRANK_EMBEDDING_MODEL=your-embedding-model
export RUSTRANK_EMBEDDING_DIMS=1536
# Supply RUSTRANK_EMBEDDING_API_KEY through your environment if needed.
./target/release/rustrank
```

For a client-launched server, add the variables to that client's server `env` configuration. Values above are examples: use your endpoint's actual model and dimensions. The base URL must not contain credentials, a query string, or a fragment.

At startup, RustRank tries to read the selected model's `max_input_tokens` from the endpoint, then sends a small test embedding request with a **five-second timeout**. It checks HTTP success, response structure, vector dimensions, and finite numeric values. Redirects are not followed. If metadata is unavailable and the variable is unset, the input limit defaults to 512. An invalid variable prevents embedding startup; when the variable differs from endpoint metadata, tool results warn and RustRank uses the smaller limit.

| Endpoint state | `index_project` | `query` |
| --- | --- | --- |
| Validated | Builds the structural index and chunk vectors; `embeddings: false` skips vectors. | Combines text/graph matches with compatible cached chunk vectors. |
| Missing settings or failed startup check | Builds the structural index, reports an MCP error with a warning, and skips embeddings. | Uses text and graph matching. |
| Request fails after startup | Reports the failed chunks, percentage coverage and an MCP error result while preserving structural results. | Falls back to text and graph matching if the query embedding fails. |

Correct the endpoint/settings and **restart RustRank** to retry startup validation. Other tools remain available throughout.

Source is split at function boundaries; oversized functions and module-level text are split further into chunks of at most **4,096 UTF-8 bytes and 80 lines**. Before sending a chunk or query, RustRank divides inputs into UTF-8 pieces no larger than the effective token limit minus eight bytes. This conservative bound leaves room for model special tokens without requiring a model-specific tokenizer. It averages the returned vectors, weighted by piece length, into one vector for the source chunk or query. Semantic results retain the matching file, symbol where available, and starting line.

`index_project` returns `embedding_coverage` and writes the current run's percentage to `.rustrank/index/v1/embedding_coverage.json` as indexing progresses. Each later tool result includes a warning and a rerun instruction while coverage is below 100%. Run `index_project` again with the same `repo_path`, `force_rebuild: false`, and `clean_stale: true` after correcting endpoint errors. Valid vectors are reused; missing vectors are retried. Each run rescans the current source set, adds new or changed chunks, and excludes deleted or changed source vectors from semantic search. CLI indexing exits nonzero when embedding chunks fail.

Cached vectors are matched to their source content, endpoint, model, and dimensions. Changed/deleted files and incompatible or legacy whole-file vectors are ignored. **Re-index after upgrading from whole-file embeddings.** Old vector files can remain on disk without affecting results.

When embeddings are enabled, source chunks and query text are sent to the configured endpoint. API keys are not written to the embedding cache.

### HTTP environment reference

| Variable | Default | Behavior |
| --- | --- | --- |
| `RUSTRANK_TRANSPORT` | `stdio` | `http`, `streamable_http`, or `streamable-http` select HTTP. Other values select stdio. |
| `RUSTRANK_LISTEN_ADDR` | Unset | Full IP socket address, such as `127.0.0.1:63477`; overrides host/port. |
| `RUSTRANK_HOST` | `127.0.0.1` | Bind IP address, not a DNS hostname. Docker defaults to `0.0.0.0`. |
| `RUSTRANK_PORT` | `63477` | Used when the full listen address is unset. |
| `RUSTRANK_MCP_PATH` | `/mcp` | Leading slash is added and trailing slashes removed; `/` and `/healthz` are rejected. |
| `RUSTRANK_ALLOWED_HOSTS` | Loopback and bound address values | Comma-separated hostnames/IPs/authorities added to the allowed list. |
| `RUSTRANK_ALLOWED_ORIGINS` | Unset | Comma-separated allowed origins; empty disables Origin validation. |
| `RUSTRANK_DISABLE_HOST_CHECK` | `false` | `true`, `1`, `yes`, or `on` disable host checks. |

Legacy `RUSTANK_*` spellings are accepted for transport/HTTP variables, not embedding variables. RustRank's current diagnostics are written directly to stderr; `RUST_LOG` does not configure a RustRank logging filter.

## Standalone CLI

The CLI indexes repositories without an MCP client:

```bash
./target/release/rustrank index-project \
  --repo-path /absolute/path/to/repo \
  --languages python,rust \
  --force-rebuild \
  --clean-stale
```

| Option | Effect |
| --- | --- |
| `--repo-path PATH` | Required repository directory. |
| `--languages LIST` | Comma-separated language names; omit to use repository settings/detection. |
| `--force-rebuild` | Rebuild structural file facts even when hashes match. Does not force embedding vectors to regenerate. |
| `--clean-stale` | Remove obsolete structural cache entries; does not remove old embedding vectors. |
| `--embeddings` | Enable vector generation for this run. |
| `--embedding-base-url URL` | Override the embedding API base. |
| `--embedding-model MODEL` | Override the model. |
| `--embedding-dims N` | Override vector dimensions. |
| `--embedding-max-input-tokens N` | Override the input limit for CLI indexing; defaults to 512. |
| `--embedding-api-key KEY` | Supply optional bearer authentication. |

CLI embedding precedence is **explicit flags → repository `embeddings` settings → defaults**. Repository keys are `enabled`, `base_url`, `model`, `dimensions`, and `max_input_tokens`; repository-stored API keys are not read. The CLI does not consume the MCP embedding environment variables.

Defaults are embeddings disabled, base `https://api.phrk.org/v1`, model `text-image-embedding`, and 1,536 dimensions. Set your own endpoint details explicitly when enabling CLI embeddings:

```bash
./target/release/rustrank index-project \
  --repo-path /absolute/path/to/repo \
  --embeddings \
  --embedding-base-url https://api.example.com/v1 \
  --embedding-model your-embedding-model \
  --embedding-dims 1536
```

Use `--help`, `index-project --help`, or `--list-tools` to inspect the binary's interface. The CLI returns a JSON indexing summary; MCP tool names such as `query` are not CLI subcommands.

## Docker setup

Build the image from this checkout:

```bash
docker build -t rustrank:local .
```

Run HTTP against a repository mounted at `/workspace/repo`. This Linux example uses your UID/GID so RustRank can write the bind-mounted index:

```bash
docker run --rm --name rustrank \
  --user "$(id -u):$(id -g)" \
  -p 127.0.0.1:63477:63477 \
  -v "/absolute/path/to/repo:/workspace/repo" \
  rustrank:local
```

Connect to `http://127.0.0.1:63477/mcp` and pass **`/workspace/repo`** as `repo_path`. Without `--user`, the image runs as UID **10001**; give that user write access when using the default identity. Indexing and `set_config` require a writable repository mount.

The image defaults to HTTP on `0.0.0.0:63477`, uses `/workspace` as its working directory, and provides a `/healthz` health check. For stdio, use `-i -e RUSTRANK_TRANSPORT=stdio` and omit the published port.

To enable embeddings, pass the configured variables into the container:

```text
-e RUSTRANK_EMBEDDING_BASE_URL
-e RUSTRANK_EMBEDDING_MODEL
-e RUSTRANK_EMBEDDING_DIMS
-e RUSTRANK_EMBEDDING_API_KEY
```

The endpoint must be reachable **from inside the container**. This repository includes a Dockerfile; the current release workflow publishes binary archives, not container images.

## Tools and resources

The server advertises its name `rustrank`, display title **RustRank**, package version, description, GitHub website, usage instructions, and embedded PNG icon. Clients decide which metadata they display.

| Tool | Use it for |
| --- | --- |
| `index_project` | Build/refresh structural indexes, optional chunk vectors, and agent guidance. |
| `query` | Rank relevant modules, symbols, and source chunks using text, graph, and optional semantic signals. |
| `context` | Inspect a symbol's definition, callers/callees, imports, and related resources. |
| `impact` | Estimate upstream/downstream dependencies affected by a symbol or module change. |
| `detect_changes` | Map **unstaged tracked-file** changes to symbols and affected code. |
| `contextual_search` | Find literal text or regex matches with surrounding lines. |
| `smart_code_search` | Rank source matches by module importance; `num_context_lines` caps results. |
| `api_usage` | Locate examples of an API, function, method, or identifier. |
| `coderank_analysis` | Rank modules using import-graph PageRank. |
| `code_hotspots` | Find connectivity and textual/change-frequency hotspots. |
| `trace_data_flow` | Find identifier occurrences and classify textual usage patterns. |
| `trace_feature_impl` | Map feature keywords to files and coarse code layers. |
| `trace_dep_impact` | Find direct import dependents of a module. |
| `error_patterns` | Find error-handling patterns, optional antipatterns, and Git history signals. |
| `perf_bottleneck` | Find simple performance-pattern matches or specified focus strings. |
| `exec_paths` | Inspect branches, loops, and optional call context inside a function. |
| `execute_paths` | Compatibility alias for `exec_paths`. |
| `get_config` | Read repository JSON configuration. |
| `set_config` | Write a JSON value at a top-level or dotted configuration key. |

Consult your client's `tools/list` schemas for argument names and required fields. Tool execution failures set `isError: true`; successful indexing can still contain warnings, so inspect the returned summary.

Clients supporting MCP resources can read:

```text
rustrank://repo/current/context
rustrank://repo/current/schema
rustrank://repo/current/modules
rustrank://repo/current/module/{name}
rustrank://repo/current/processes
rustrank://repo/current/process/{name}
```

Successful `index_project` selects the current repository for resources; otherwise they use the server's working directory. The selection is process-wide, so shared clients can change it. Use explicit `repo_path` tool arguments when working across repositories.

## Supported languages

| Language | Config name | Source extensions |
| --- | --- | --- |
| Python | `python` | `.py` |
| Rust | `rust` | `.rs` |
| C# | `csharp` | `.cs` |
| TypeScript | `typescript` | `.ts`, `.tsx` |
| JavaScript | `javascript` | `.js`, `.jsx`, `.mjs`, `.cjs` |
| C | `c` | `.c`, `.h` |
| C++ | `cpp` | `.cpp`, `.cc`, `.cxx`, `.c++`, `.hpp`, `.hh`, `.hxx`, `.h++` |
| Go | `go` | `.go` |

Python uses RustPython/tree-sitter parsing; other supported languages use tree-sitter. Language support provides static source facts, not compiler-level type checking or execution.

## Files RustRank writes

Indexing creates or updates these files under the selected repository:

```text
.rustrank/index/v1/
  project_manifest.json
  embedding_coverage.json                   # current progress and failures
  languages/<language>/index.json
  languages/<language>/files/<content-hash>.json
  embeddings/<chunk-id>.json                  # when vectors are generated
.rustrank/skills/
  exploring.md
  impact-analysis.md
  debugging.md
  refactoring.md
AGENTS.md                                   # generated section only
```

`set_config` writes `.rustrank_config.json`. Indexing preserves manual `AGENTS.md` content outside the `<!-- rustrank-index:start -->` and `<!-- rustrank-index:end -->` markers.

Structural caches contain relative paths, symbols, imports, namespaces, hashes, graph relationships, and Git freshness information. Embedding caches contain vectors and location/model metadata. They do not store source snippets or embedding API keys. Tools may return source snippets to the MCP client when requested.

Re-index after edits to refresh persisted facts and vectors. `--force-rebuild` rebuilds structural facts; `--clean-stale` removes obsolete structural entries. Legacy or stale vector files are ignored during semantic search rather than automatically removed.

## Definitions

| Term | Meaning in RustRank |
| --- | --- |
| **MCP** | The protocol an assistant client uses to discover and call RustRank tools or read resources. |
| **Structural index** | Parsed source facts: files, definitions, imports, and their relationships. It needs no embedding service. |
| **Module** | A source file or language namespace represented in the dependency graph. |
| **Symbol** | A named definition, such as a function, class, or type, with a source location. |
| **Import graph / PageRank** | A graph of module dependencies and a ranking based on those links; useful for finding central code. |
| **Embedding** | A numeric vector returned by a model to represent source or query text for similarity comparison. |
| **Chunk** | A bounded source segment with file/line metadata, usually within a function or module-level text. |
| **Semantic search** | Matching query and source vectors by similarity, alongside text and graph signals. |
| **Manifest / shard** | The project-wide index summary / a language-specific portion of the cache. |
| **Process flow** | A heuristic call chain derived from static code relationships, not a recorded runtime trace. |
| **Cache hit** | Reuse of compatible cached facts/vectors. The indexing summary's hit/miss counters describe structural file facts. |
| **Stdio / Streamable HTTP** | MCP over a child process's input/output / MCP through an HTTP endpoint. |

## Troubleshooting and limits

| Symptom | Check |
| --- | --- |
| Server appears idle in a terminal | Stdio waits for an MCP client. Use `--help` or `index-project` for CLI operations. |
| “Embeddings unavailable” or “Embeddings skipped” | Verify the server environment, base URL, model, dimensions, and optional key; restart after correcting them. Structural indexing still works. |
| HTTP 401/403 from embedding service | Verify the optional API key and endpoint's authorization requirements. |
| Vector dimension mismatch | Set dimensions supported by the selected model; re-index after changing model/dimensions. |
| No semantic matches | Confirm startup validation passed, index with embeddings enabled, and re-index edited files. Old whole-file vectors are ignored. |
| Permission denied while indexing | Check repository permissions and Docker UID/mount settings. |
| HTTP host rejected | Add the externally visible hostname/authority to `RUSTRANK_ALLOWED_HOSTS`; use an IP address for bind settings. |
| Client still shows old tool arguments | Rebuild/update the binary the client actually launches, restart it, and refresh/reconnect the client. |
| Expected files are absent | Check supported extensions, enabled languages, path overrides, excludes, and UTF-8 encoding. |

RustRank's analysis is static and partly heuristic. Call graphs, inferred layers, execution paths, data-flow labels, and performance findings need verification in source; they are not proof of runtime behavior. Large repositories may take time because embedding requests are made sequentially per uncached chunk. Many analysis tools also parse current source rather than reading every result from the persistent cache.

## Development

The Cargo workspace contains one crate in `src/`. Key code is in `context.rs` (parsing), `index.rs` (persistent index), `embedding_chunks.rs` (chunking), `embeddings.rs` (endpoint/cache/scoring), `project_config.rs` (repository settings), and `tools/` (MCP and CLI handlers).

Run the standard Rust checks:

```bash
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

[`.githooks/pre-push`](.githooks/pre-push) contains the repository's Rust check sequence. For an HTTP smoke test, start the server as described above, then use Python 3 in another shell:

```bash
python3 -m venv .venv
. .venv/bin/activate
python scripts/smoke_http_json.py --url http://127.0.0.1:63477/mcp
```

The script uses Python's standard library. It creates a temporary fixture, checks all 19 tools, and exercises resources. With a configured embedding endpoint it also exercises vector generation; otherwise it checks structural fallback.

The [Forgejo release workflow](.forgejo/workflows/release.yml) publishes when the application version changes on `main`. Manual choices are `Major` (increment the middle component), `minor` (increment the final component), and `retry` (publish the current version). Its configuration is in [`.forgejo/release.json`](.forgejo/release.json); release notes/artifacts are available on [Releases](https://git.phrk.org/pub/RustRank/releases).

Further implementation and validation material:

- [Implementation notes](docs/IMPLEMENTATION.md)
- [Protocol and behavior specification](docs/SPEC.md)
- [Validation against external C/C++/Go repositories](README_REAL_REPO_VALIDATION.md)
