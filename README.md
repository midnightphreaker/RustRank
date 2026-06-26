# RustRank

RustRank is a Rust MCP server for repository analysis. It exposes search,
CodeRank, trace, analysis, and local configuration tools over the `rmcp` server
framework.

## Tools

- `contextual_search`
- `smart_code_search`
- `api_usage`
- `coderank_analysis`
- `code_hotspots`
- `trace_data_flow`
- `trace_feature_impl`
- `trace_dep_impact`
- `error_patterns`
- `perf_bottleneck`
- `exec_paths`
- `execute_paths` (alias for `exec_paths`)
- `get_config`
- `set_config`

## Development

```bash
cargo test --workspace
cargo clippy --all-targets --all-features
```

Install the repository pre-push hook:

```bash
git config core.hooksPath .githooks
```

Run the binary in stdio MCP mode:

```bash
cargo run -p rustrank
```

Run Streamable HTTP on `/mcp`:

```bash
RUSTRANK_TRANSPORT=streamable_http RUSTRANK_LISTEN_ADDR=127.0.0.1:63477 cargo run -p rustrank
```

The implementation also accepts the `RUSTANK_TRANSPORT` and `RUSTANK_LISTEN_ADDR`
spellings used by the original spec.

List registered tool names without starting the transport:

```bash
cargo run -p rustrank -- --list-tools
```
