# AGENTS.md

<!-- rustrank-index:start -->
## RustRank Indexed Codebase

RustRank indexed this repository with language-specific analyzers. Use the persistent cache and project manifest for repository-level symbol/import context before broad code changes.

Persistent index cache: `.rustrank/index/v1/`
Project manifest: `.rustrank/index/v1/project_manifest.json`

Language index shards:

| Language | Files | Symbols | Imports |
| --- | ---: | ---: | ---: |
| python | 1 | 11 | 10 |
| rust | 16 | 257 | 52 |

The cache stores per-file symbols, imports, declared namespaces, content hashes, and language metadata. It does not store source lines, snippets, absolute paths, or timestamps. Re-run `index_project` after source changes to refresh this section and the persistent cache.
<!-- rustrank-index:end -->
