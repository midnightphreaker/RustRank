# RustRank Real-Repo Validation

This checklist validates RustRank against external C, C++, and Go repositories after parser, indexing, search, or graph-ranking changes. It is intentionally separate from the unit and integration test suite so large third-party repositories are not required for normal development.

## Repositories

The latest validation run used these pinned commits:

| Repository | Purpose | Pinned commit | Indexed files | Expected languages |
| --- | --- | --- | ---: | --- |
| `libexpat/libexpat` | C headers, C sources, macro-wrapped prototypes | `0ac39627c7942f760e27cad67c8a6f09864381fe` | 84 | python: 2, c: 81, cpp: 1 |
| `fmtlib/fmt` | C++ headers, C++ sources, macro-heavy declarations | `81516a20d975483bc55687cd04161f2ef7d38a63` | 77 | python: 4, javascript: 1, c: 26, cpp: 46 |
| `gin-gonic/gin` | Go grouped imports, aliases, function type declarations | `34dac209ffb6ef85cc78c5d217bbb7ad001d68fd` | 99 | go: 99 |

## Clone Commands

```bash
mkdir -p /tmp/rustrank-real-repos

git clone https://github.com/libexpat/libexpat.git /tmp/rustrank-real-repos/libexpat
git -C /tmp/rustrank-real-repos/libexpat checkout 0ac39627c7942f760e27cad67c8a6f09864381fe

git clone https://github.com/fmtlib/fmt.git /tmp/rustrank-real-repos/fmt
git -C /tmp/rustrank-real-repos/fmt checkout 81516a20d975483bc55687cd04161f2ef7d38a63

git clone https://github.com/gin-gonic/gin.git /tmp/rustrank-real-repos/gin
git -C /tmp/rustrank-real-repos/gin checkout 34dac209ffb6ef85cc78c5d217bbb7ad001d68fd
```

## Index Commands

Run from the RustRank repository root:

```bash
cargo run -p rustrank -- index-project \
  --repo-path /tmp/rustrank-real-repos/libexpat \
  --force-rebuild \
  --clean-stale

cargo run -p rustrank -- index-project \
  --repo-path /tmp/rustrank-real-repos/fmt \
  --force-rebuild \
  --clean-stale

cargo run -p rustrank -- index-project \
  --repo-path /tmp/rustrank-real-repos/gin \
  --force-rebuild \
  --clean-stale
```

## Pass Criteria

Each command should produce JSON with `scanned_files == indexed_files`, an empty `warnings` array, and the expected language/file counts listed above.

Representative facts should be present in the generated per-file caches:

| Repository | Expected facts |
| --- | --- |
| `libexpat/libexpat` | `XML_ParserCreate` and `XML_SetEncoding` as `func` symbols in `expat/lib/expat.h` and `expat/lib/xmlparse.c` |
| `fmtlib/fmt` | `fmt_vformat` and `convert_c_format_args` as `func` symbols in `src/fmt-c.cc`; imports of `fmt/fmt-c.h` and `fmt/base.h` |
| `gin-gonic/gin` | `Default` and `HandlerFunc` as `func` symbols in `gin.go`; imports of `net/http` and `github.com/gin-gonic/gin/internal/bytesconv` |

For search/ranking checks, `smart_code_search` for `XML_ParserCreate` in libexpat should return non-empty C results with non-zero scores. `code_hotspots` can be slow on large real repositories because it may inspect git blame for many connected modules; use fixture-level integration tests for the routine hotspot regression.

## Troubleshooting

- `rustrank --help` and `rustrank index-project --help` are CLI paths and should exit successfully with usage text.
- Running `rustrank` with no arguments starts the default MCP stdio server. If a shell invocation appears to wait for input, check whether a CLI subcommand was omitted.
- If an already-running MCP server reports that C, C++, or Go are unsupported, restart that MCP server so it picks up the rebuilt RustRank binary.
- If a repository was indexed before parser changes, rerun with `--force-rebuild --clean-stale`.
- If generated `.rustrank` cache files look stale, remove the target repository's `.rustrank/index/v1/` directory and rerun the index command.

## Rerun After Parser Changes

1. Run the normal RustRank checks:

   ```bash
   cargo test -p rustrank
   cargo clippy -p rustrank --all-targets --all-features -- -D warnings
   ```

2. Re-run the three real-repo index commands above.
3. Inspect the JSON summaries for zero warnings and expected file counts.
4. Spot-check representative symbols/imports in `.rustrank/index/v1/languages/*/files/*.json`.
5. Restart any live MCP server before validating through MCP tools.
