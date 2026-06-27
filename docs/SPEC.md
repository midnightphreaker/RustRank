# RustRank MCP Server Spec

RustRank is a Rust MCP server for repository analysis over stdio or stateless
Streamable HTTP with JSON responses. It provides 15 tools for indexing, search,
CodeRank, dependency tracing, execution-path inspection, pattern analysis, and
per-repository configuration.

## Supported Languages

RustRank detects supported languages by scanning non-ignored files and matching
extensions:

| Language | Extensions |
| --- | --- |
| Python | `.py` |
| Rust | `.rs` |
| C# | `.cs` |
| TypeScript | `.ts`, `.tsx` |
| JavaScript | `.js`, `.jsx`, `.mjs`, `.cjs` |

Ignored directories include `.git`, `.rustrank`, `target`, `node_modules`,
`dist`, and `build`.

## Project Configuration

Repository configuration is stored at:

```text
<repo_path>/.rustrank_config.json
```

The raw `get_config` and `set_config` tools remain key/value JSON operations.
The typed language configuration lives under the top-level `languages` key:

```json
{
  "languages": {
    "enabled": ["python", "rust", "csharp", "typescript", "javascript"]
  }
}
```

If `languages.enabled` is missing or empty, RustRank auto-detects languages
from current source files. Unknown language names are ignored and reported as
warnings by indexing.

## Persistent Index

`index_project` creates deterministic, repo-local JSON under:

```text
<repo_path>/.rustrank/index/v1/
```

Layout:

```text
.rustrank/index/v1/project_manifest.json
.rustrank/index/v1/languages/<language>/index.json
.rustrank/index/v1/languages/<language>/files/<blake3-hash>.json
```

The per-file cache stores:

- schema/cache/extractor version
- relative source path
- language
- module name
- BLAKE3 content hash and file size
- extracted symbols
- extracted imports
- C# declared namespaces

The cache does not store source lines, snippets, absolute paths, timestamps, or
secrets. The project manifest is the aggregate index across language indexes:
it references every per-language shard and records resolved/unresolved import
edges across the indexed modules.

After a successful index, RustRank also creates or amends
`<repo_path>/AGENTS.md`. The generated section is bounded by
`<!-- rustrank-index:start -->` and `<!-- rustrank-index:end -->`, summarizes the
indexed languages and cache locations, and preserves manual content outside the
markers.
it references each language shard and contains resolved import edges plus
unresolved imports.

## Tools

| Tool | Purpose |
| --- | --- |
| `index_project` | Precompute per-language caches, project manifest, and AGENTS index summary |
| `contextual_search` | Search repository files with line context |
| `smart_code_search` | Search code and rank results by module importance |
| `api_usage` | Find API/function usage examples |
| `coderank_analysis` | Rank modules using import-graph PageRank |
| `code_hotspots` | Find important and frequently referenced modules |
| `trace_data_flow` | Trace an identifier through source files |
| `trace_feature_impl` | Map feature keywords across code layers |
| `trace_dep_impact` | Find direct import dependents for a module |
| `error_patterns` | Find error-handling patterns and antipatterns |
| `perf_bottleneck` | Detect simple performance bottleneck patterns |
| `exec_paths` | Trace branches and loops inside a function |
| `execute_paths` | Alias for `exec_paths` |
| `get_config` | Read repository JSON configuration |
| `set_config` | Set a top-level repository config value |

## CLI

Default execution starts the selected MCP transport. Utility commands exit
without starting a transport:

```bash
cargo run -p rustrank -- --list-tools
cargo run -p rustrank -- index-project --repo-path /path/to/repo
```

`index-project` accepts `--languages`, `--force-rebuild`, and `--clean-stale`.
It writes JSON to stdout and structured JSON errors to stderr for usage errors.
