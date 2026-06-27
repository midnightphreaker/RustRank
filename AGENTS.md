# AGENTS.md

<!-- rustrank-index:start -->
## RustRank Indexed Codebase

RustRank indexed this repository with language-specific analyzers. Use the persistent cache and project manifest for repository-level symbol/import context before broad code changes.

Persistent index cache: `.rustrank/index/v1/`
Project manifest: `.rustrank/index/v1/project_manifest.json`

MCP resources: `rustrank://repo/current/context`, `rustrank://repo/current/schema`, `rustrank://repo/current/modules`, `rustrank://repo/current/module/{name}`, `rustrank://repo/current/processes`, and `rustrank://repo/current/process/{name}`.

Agent-facing tools: `context`, `impact`, `detect_changes`, and `query`. Use `context` before editing a symbol, `impact` before changing shared APIs, `detect_changes` before final review, and `query` for graph-aware repository search.

Workflow docs: `.rustrank/skills/exploring.md`, `.rustrank/skills/impact-analysis.md`, `.rustrank/skills/debugging.md`, `.rustrank/skills/refactoring.md`.

Language index shards:

| Language | Files | Symbols | Imports |
| --- | ---: | ---: | ---: |
| python | 1 | 20 | 12 |
| rust | 19 | 376 | 71 |

The cache stores per-file symbols, imports, declared namespaces, content hashes, graph nodes, graph edges, and git freshness metadata. It does not store source lines, snippets, or absolute paths. Re-run `index_project` after source changes to refresh this section and the persistent cache.
<!-- rustrank-index:end -->
