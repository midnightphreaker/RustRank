# RustRank

RustRank is an MCP server for repository analysis. It exposes search, CodeRank,
trace, analysis, execution-path, and per-repository configuration tools over
stdio or Streamable HTTP.

The HTTP transport is stateless Streamable HTTP with JSON responses only. It
does not use SSE.

## Supported Languages

RustRank parses and ranks:

- Python
- Rust
- C#
- TypeScript and TSX
- JavaScript and JSX

## MCP Tools

- `contextual_search`: search repository files with line context
- `smart_code_search`: search code and rank results by module importance
- `api_usage`: find examples of an API or function call
- `coderank_analysis`: rank modules with import-graph PageRank
- `code_hotspots`: find highly connected modules
- `trace_data_flow`: trace an identifier through a repository
- `trace_feature_impl`: map feature keywords across code layers
- `trace_dep_impact`: find direct dependency impact for a module
- `error_patterns`: find error-handling patterns and antipatterns
- `perf_bottleneck`: detect simple performance bottleneck patterns
- `exec_paths`: trace branch and loop execution paths for a function
- `execute_paths`: alias for `exec_paths`
- `get_config`: read RustRank JSON configuration for a repository
- `set_config`: write a RustRank JSON configuration value for a repository

List registered tools without starting a transport:

```bash
cargo run -p rustrank -- --list-tools
```

## Transports

### Stdio

Stdio is the default transport:

```bash
cargo run -p rustrank
```

### Streamable HTTP, No SSE

Run the remote MCP server on `POST /mcp`:

```bash
RUSTRANK_TRANSPORT=streamable_http \
RUSTRANK_HOST=127.0.0.1 \
RUSTRANK_PORT=63477 \
cargo run -p rustrank
```

The server also exposes `GET /healthz` for process and container health checks.
The MCP endpoint is stateless and POST-only. `GET /mcp` and `DELETE /mcp` are
not used because this mode does not create HTTP sessions or SSE streams.

MCP HTTP clients should send:

- `Content-Type: application/json`
- `Accept: application/json, text/event-stream`
- `MCP-Protocol-Version: 2025-06-18` or another RMCP-supported MCP protocol version

Responses from RustRank's HTTP mode are `application/json`, not
`text/event-stream`.

## Docker Setup

Prerequisites:

- Docker Engine or a compatible Docker runtime
- A repository directory on the host to mount into the container
- A host port to publish, default `63477`

The image runs RustRank as a no-SSE Streamable HTTP MCP server by default. It
binds to `0.0.0.0:63477` inside the container and serves MCP at `/mcp`.

### 1. Build the Image

From the repository root:

```bash
docker build -t rustrank:local .
```

### 2. Run Locally

This exposes RustRank only on the host loopback interface and mounts the current
repository at `/workspace/repo`:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v "$PWD:/workspace/repo" \
  rustrank:local
```

Use this MCP URL from clients running on the same host:

```text
http://127.0.0.1:63477/mcp
```

Check the process health endpoint:

```bash
curl -fsS http://127.0.0.1:63477/healthz
```

Follow logs:

```bash
docker logs -f rustrank
```

Stop the server:

```bash
docker stop rustrank
```

### 3. Run Against a Specific Host Repository

Mount any host repository under `/workspace` and pass that container path to
RustRank MCP tools as `repo_path`:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v /host/path/to/repo:/workspace/repo \
  rustrank:local
```

For read-only analysis, mount the repository read-only:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:63477:63477 \
  -v /host/path/to/repo:/workspace/repo:ro \
  rustrank:local
```

Use a read-write mount when calling `set_config`, because that tool writes
`/workspace/repo/.rustrank_config.json`.

### 4. Publish for Remote Clients

For remote access, publish the port on all host interfaces and allow the public
host name or IP in RMCP Host validation:

```bash
docker run --rm \
  --name rustrank \
  -p 63477:63477 \
  -e RUSTRANK_ALLOWED_HOSTS="rustrank.example.com,rustrank.example.com:63477" \
  -v /srv/repos/my-repo:/workspace/my-repo \
  rustrank:local
```

If a reverse proxy rewrites the `Host` header, set `RUSTRANK_ALLOWED_HOSTS` to
the host value that reaches the container.

### 5. Custom Port or Path

Change the published host port without changing the container port:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:8080:63477 \
  -v "$PWD:/workspace/repo" \
  rustrank:local
```

The MCP URL becomes:

```text
http://127.0.0.1:8080/mcp
```

Change the container listen port and MCP path with environment variables:

```bash
docker run --rm \
  --name rustrank \
  -p 127.0.0.1:9000:9000 \
  -e RUSTRANK_PORT=9000 \
  -e RUSTRANK_MCP_PATH=/rustrank \
  -v "$PWD:/workspace/repo" \
  rustrank:local
```

## Docker Volumes and Data

RustRank does not use a separate database. It reads repositories directly from
the filesystem path passed to each MCP tool.

Mount repositories under `/workspace`:

```bash
-v /host/path/to/repo:/workspace/repo
```

Use a read-write mount when calling `set_config`. That tool stores repository
configuration in:

```text
<repo_path>/.rustrank_config.json
```

Read-only mounts are fine for analysis-only tools:

```bash
-v /host/path/to/repo:/workspace/repo:ro
```

The container runs as UID `10001`, user `rustrank`. Mounted directories must be
readable by that UID, and writable by that UID if `set_config` is used.

## Environment Variables

Canonical variables use the `RUSTRANK_` prefix. The original misspelled
`RUSTANK_` aliases are still accepted for compatibility.

| Variable | Default | Description |
| --- | --- | --- |
| `RUSTRANK_TRANSPORT` | `stdio` locally, `streamable_http` in Docker | `stdio`, `http`, `streamable_http`, or `streamable-http`. |
| `RUSTRANK_LISTEN_ADDR` | unset | Full HTTP bind address, such as `127.0.0.1:63477`. Takes precedence over host and port. |
| `RUSTRANK_HOST` | `127.0.0.1` locally, `0.0.0.0` in Docker | HTTP bind host when `RUSTRANK_LISTEN_ADDR` is unset. |
| `RUSTRANK_PORT` | `63477` | HTTP bind port when `RUSTRANK_LISTEN_ADDR` is unset. |
| `RUSTRANK_MCP_PATH` | `/mcp` | MCP HTTP path. Values without a leading slash are normalized, for example `mcp` becomes `/mcp`. |
| `RUSTRANK_ALLOWED_HOSTS` | loopback plus bound host | Comma-separated allowed `Host` values. Use hostnames, IPs, or `host:port` authorities. |
| `RUSTRANK_ALLOWED_ORIGINS` | unset | Comma-separated allowed browser `Origin` values, such as `https://app.example.com`. Empty means Origin validation is disabled. |
| `RUSTRANK_DISABLE_HOST_CHECK` | `false` | Set to `true`, `1`, `yes`, or `on` to disable RMCP Host validation. Use only on trusted networks. |
| `RUST_LOG` | unset locally, `info` in Docker | Standard Rust logging filter used by compatible dependencies. |

Accepted legacy aliases:

- `RUSTANK_TRANSPORT`
- `RUSTANK_LISTEN_ADDR`
- `RUSTANK_HOST`
- `RUSTANK_PORT`
- `RUSTANK_MCP_PATH`
- `RUSTANK_ALLOWED_HOSTS`
- `RUSTANK_ALLOWED_ORIGINS`
- `RUSTANK_DISABLE_HOST_CHECK`

## Network and Proxy Notes

- Container port: `63477/tcp`
- Default MCP path: `/mcp`
- Health path: `/healthz`
- Docker default bind inside the container: `0.0.0.0:63477`
- Local binary default bind: `127.0.0.1:63477`
- HTTP mode is stateless, JSON-response Streamable HTTP. It does not create SSE
  streams and does not require an MCP session ID.
- Host validation is enabled by default to reduce DNS rebinding risk. Add
  public DNS names, reverse-proxy hostnames, or published `host:port` values to
  `RUSTRANK_ALLOWED_HOSTS`.
- Origin validation is disabled unless `RUSTRANK_ALLOWED_ORIGINS` is set.
  Missing `Origin` headers are accepted.

## Smoke Test

Test a running HTTP server from the host:

```bash
scripts/smoke_http_json.py --url http://127.0.0.1:63477/mcp
```

Test the Docker image locally:

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

The smoke test initializes the MCP endpoint, verifies `tools/list`, rejects SSE
responses, and calls every RustRank MCP tool.

## Development

Run the test suite:

```bash
cargo test --workspace
```

Run clippy:

```bash
cargo clippy --all-targets --all-features
```

Install the repository pre-push hook:

```bash
git config core.hooksPath .githooks
```
