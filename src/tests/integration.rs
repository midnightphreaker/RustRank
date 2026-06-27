use rustrank::context::{Context, DefKind, Language, supported_source_files};
use rustrank::tools::ALL_TOOLS;
use rustrank::tools::analysis::{error_patterns, exec_paths, perf_bottleneck};
use rustrank::tools::code_rank::{code_hotspots, coderank_analysis};
use rustrank::tools::config::{get_config, set_config};
use rustrank::tools::index::index_project;
use rustrank::tools::search::{api_usage, contextual_search, smart_code_search};
use rustrank::tools::trace::{trace_data_flow, trace_dep_impact, trace_feature_impl};

mod fixtures;

use fixtures::fixture;

#[test]
fn router_registers_fifteen_tools() {
    assert_eq!(ALL_TOOLS.len(), 15);
    assert!(ALL_TOOLS.contains(&"exec_paths"));
    assert!(ALL_TOOLS.contains(&"execute_paths"));
    assert!(ALL_TOOLS.contains(&"index_project"));
}

#[test]
fn language_from_path_recognizes_all_supported_extensions() {
    assert_eq!(
        Language::from_path(std::path::Path::new("pkg/core.py")),
        Some(Language::Python)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("src/lib.rs")),
        Some(Language::Rust)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("app/Controller.cs")),
        Some(Language::CSharp)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("web/auth.ts")),
        Some(Language::TypeScript)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("web/view.tsx")),
        Some(Language::TypeScript)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("web/auth.js")),
        Some(Language::JavaScript)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("web/view.jsx")),
        Some(Language::JavaScript)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("web/browser.mjs")),
        Some(Language::JavaScript)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("web/legacy.cjs")),
        Some(Language::JavaScript)
    );
    assert_eq!(Language::from_path(std::path::Path::new("README.md")), None);
    assert_eq!(
        Language::from_path(std::path::Path::new("package.json")),
        None
    );
    assert_eq!(Language::from_path(std::path::Path::new("Makefile")), None);
}

#[test]
fn context_parse_cache_extracts_imports_and_defs() {
    let fixture = fixture();
    let ctx = Context::new(fixture.path().to_path_buf());

    let first = ctx.get_or_parse("pkg/core.py".to_string()).expect("parse");
    let second = ctx.get_or_parse("pkg/core.py".to_string()).expect("cached");

    assert_eq!(ctx.cache_len(), 1);
    assert_eq!(first.imports.len(), 2);
    assert!(first.defs.iter().any(|def| def.name == "authenticate"));
    assert!(second.defs.iter().any(|def| def.kind == DefKind::Func));
}

#[test]
fn context_parse_all_extracts_supported_languages() {
    let fixture = fixture();
    let ctx = Context::new(fixture.path().to_path_buf());
    let modules = ctx.parse_all().expect("parse all");

    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::Python)
    );
    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::Rust)
    );
    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::CSharp)
    );
    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::TypeScript)
    );
    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::JavaScript)
    );

    let rust = modules
        .iter()
        .find(|module| module.path.ends_with("src/lib.rs"))
        .expect("rust module");
    assert!(
        rust.imports
            .iter()
            .any(|import| import.module == "crate.service")
    );
    assert!(rust.defs.iter().any(|def| def.name == "login_user"));

    let csharp = modules
        .iter()
        .find(|module| module.path.ends_with("app/Controller.cs"))
        .expect("csharp module");
    assert!(
        csharp
            .imports
            .iter()
            .any(|import| import.module == "App.Services")
    );
    assert!(csharp.defs.iter().any(|def| def.name == "LoginController"));

    let typescript = modules
        .iter()
        .find(|module| module.path.ends_with("web/auth.ts"))
        .expect("typescript module");
    assert!(
        typescript
            .imports
            .iter()
            .any(|import| import.module == "./logger")
    );
    assert!(typescript.defs.iter().any(|def| def.name == "loginUser"));

    let javascript = modules
        .iter()
        .find(|module| module.path.ends_with("web/auth.js"))
        .expect("javascript module");
    assert!(
        javascript
            .imports
            .iter()
            .any(|import| import.module == "./format")
    );
    assert!(javascript.defs.iter().any(|def| def.name == "loginBrowser"));
}

#[test]
fn supported_source_files_ignores_persistent_rustrank_data() {
    let fixture = fixture();
    let data_dir = fixture
        .path()
        .join(".rustrank/index/v1/languages/python/files");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::write(data_dir.join("fake.py"), "def should_not_parse(): pass").expect("cache file");

    let files = supported_source_files(fixture.path()).expect("source files");

    assert!(
        !files
            .iter()
            .any(|(path, _)| path.to_string_lossy().contains(".rustrank"))
    );
}

#[test]
fn contextual_search_finds_regex_matches_with_context() {
    let fixture = fixture();
    let rows = contextual_search(
        fixture.path().to_str().unwrap(),
        "authenticate",
        Some("py"),
        false,
        1,
    )
    .expect("search");

    assert!(rows.iter().any(|row| row.file.ends_with("pkg/core.py")));
    assert!(rows.iter().any(|row| row.snippet.contains("authenticate")));
}

#[test]
fn contextual_search_returns_empty_for_no_match() {
    let fixture = fixture();
    let rows = contextual_search(
        fixture.path().to_str().unwrap(),
        "definitely_missing",
        Some("py"),
        false,
        0,
    )
    .expect("search");

    assert!(rows.is_empty());
}

#[test]
fn smart_code_search_orders_ranked_results() {
    let fixture = fixture();
    let rows = smart_code_search(fixture.path().to_str().unwrap(), "login", 1, 5).expect("smart");

    assert!(!rows.is_empty());
    assert!(rows[0].score >= rows.last().unwrap().score);
}

#[test]
fn smart_code_search_finds_supported_language_files() {
    let fixture = fixture();
    let rows = smart_code_search(fixture.path().to_str().unwrap(), "loginUser", 1, 10)
        .expect("smart multi-language");

    assert!(rows.iter().any(|row| row.file.ends_with("web/auth.ts")));
}

#[test]
fn api_usage_groups_examples() {
    let fixture = fixture();
    let rows = api_usage(fixture.path().to_str().unwrap(), "authenticate", 10, true).expect("api");

    assert!(rows.iter().any(|row| row.pattern_key.contains("call")));
}

#[test]
fn coderank_identifies_imported_modules() {
    let fixture = fixture();
    let rows = coderank_analysis(fixture.path().to_str().unwrap(), 5, None, true).expect("rank");

    assert!(!rows.is_empty());
    let sum: f64 = rows.iter().map(|row| row.score).sum();
    assert!(sum > 0.0);
}

#[test]
fn coderank_external_modules_flag_controls_nonlocal_imports() {
    let fixture = fixture();
    let local_only =
        coderank_analysis(fixture.path().to_str().unwrap(), 20, None, false).expect("local rank");
    let with_external =
        coderank_analysis(fixture.path().to_str().unwrap(), 20, None, true).expect("external rank");

    assert!(!local_only.iter().any(|row| row.module == "time"));
    assert!(with_external.iter().any(|row| row.module == "time"));
}

#[test]
fn coderank_includes_supported_language_modules() {
    let fixture = fixture();
    let rows =
        coderank_analysis(fixture.path().to_str().unwrap(), 50, None, false).expect("rank all");

    assert!(rows.iter().any(|row| row.module == "src.lib"));
    assert!(rows.iter().any(|row| row.module == "app.Controller"));
    assert!(rows.iter().any(|row| row.module == "web.auth"));
}

#[test]
fn coderank_resolves_local_imports_by_language() {
    let fixture = fixture();
    let rows =
        coderank_analysis(fixture.path().to_str().unwrap(), 50, None, false).expect("rank all");

    assert!(
        rows.iter()
            .any(|row| row.module == "pkg.models" && row.depth > 0)
    );
    assert!(
        rows.iter()
            .any(|row| row.module == "src.service" && row.depth > 0)
    );
    assert!(
        rows.iter()
            .any(|row| row.module == "app.AuthService" && row.depth > 0)
    );
    assert!(
        rows.iter()
            .any(|row| row.module == "web.logger" && row.depth > 0)
    );
    assert!(
        rows.iter()
            .any(|row| row.module == "web.format" && row.depth > 0)
    );
}

#[test]
fn enabled_language_config_filters_parse_and_tools() {
    let fixture = fixture();
    set_config(
        fixture.path().to_str().unwrap(),
        "languages",
        serde_json::json!({ "enabled": ["python", "rust"] }),
    )
    .expect("set languages");

    let ctx = Context::new(fixture.path().to_path_buf());
    let modules = ctx.parse_all().expect("parse filtered");

    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::Python)
    );
    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::Rust)
    );
    assert!(
        !modules
            .iter()
            .any(|module| module.language == Language::CSharp)
    );
    assert!(
        !modules
            .iter()
            .any(|module| matches!(module.language, Language::TypeScript | Language::JavaScript))
    );

    let rank_rows =
        coderank_analysis(fixture.path().to_str().unwrap(), 50, None, false).expect("rank");
    assert!(rank_rows.iter().any(|row| row.module == "pkg.core"));
    assert!(rank_rows.iter().any(|row| row.module == "src.lib"));
    assert!(!rank_rows.iter().any(|row| row.module.starts_with("app.")));
    assert!(!rank_rows.iter().any(|row| row.module.starts_with("web.")));
}

#[test]
fn index_project_generates_language_shards_and_project_manifest() {
    let fixture = fixture();

    let summary =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");

    assert!(summary.scanned_files >= 10);
    assert_eq!(summary.indexed_files, summary.scanned_files);
    assert!(summary.cache_misses > 0);
    assert!(
        summary
            .project_manifest
            .ends_with(".rustrank/index/v1/project_manifest.json")
    );

    let manifest_path = fixture
        .path()
        .join(".rustrank/index/v1/project_manifest.json");
    assert!(manifest_path.exists());

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path).expect("manifest"))
            .expect("manifest json");
    assert_eq!(manifest["header"]["schema_version"], 1);
    assert!(manifest["languages"].as_array().unwrap().len() >= 5);
    assert!(
        manifest["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |edge| edge["source_module"] == "pkg.core" && edge["target_module"] == "pkg.models"
            )
    );

    let cache_text = std::fs::read_to_string(
        fixture
            .path()
            .join(".rustrank/index/v1/languages/python/index.json"),
    )
    .expect("python shard");
    assert!(!cache_text.contains("source_lines"));
    assert!(!cache_text.contains(fixture.path().to_str().unwrap()));
}

#[test]
fn index_project_creates_agents_md_with_index_summary() {
    let fixture = fixture();

    index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");

    let agents_path = fixture.path().join("AGENTS.md");
    let agents = std::fs::read_to_string(agents_path).expect("agents file");
    assert!(agents.contains("# AGENTS.md"));
    assert!(agents.contains("## RustRank Indexed Codebase"));
    assert!(agents.contains("Persistent index cache: `.rustrank/index/v1/`"));
    assert!(agents.contains("Project manifest: `.rustrank/index/v1/project_manifest.json`"));
    assert!(agents.contains("| python |"));
    assert!(agents.contains("| rust |"));
    assert!(agents.contains("| csharp |"));
    assert!(agents.contains("| typescript |"));
    assert!(agents.contains("| javascript |"));
}

#[test]
fn index_project_amends_existing_agents_md_without_duplicating_section() {
    let fixture = fixture();
    std::fs::write(
        fixture.path().join("AGENTS.md"),
        "# AGENTS.md\n\nKeep this instruction.\n\n<!-- rustrank-index:start -->\nold summary\n<!-- rustrank-index:end -->\n",
    )
    .expect("seed agents file");

    index_project(fixture.path().to_str().unwrap(), None, true, true).expect("first index");
    index_project(fixture.path().to_str().unwrap(), None, false, true).expect("second index");

    let agents = std::fs::read_to_string(fixture.path().join("AGENTS.md")).expect("agents file");
    assert!(agents.contains("Keep this instruction."));
    assert!(!agents.contains("old summary"));
    assert_eq!(agents.matches("<!-- rustrank-index:start -->").count(), 1);
    assert_eq!(agents.matches("<!-- rustrank-index:end -->").count(), 1);
    assert_eq!(agents.matches("## RustRank Indexed Codebase").count(), 1);
}

#[test]
fn index_project_reuses_unchanged_file_hashes() {
    let fixture = fixture();
    let first =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("first index");
    let second =
        index_project(fixture.path().to_str().unwrap(), None, false, true).expect("second index");

    assert_eq!(first.scanned_files, second.scanned_files);
    assert!(second.cache_hits > 0);
    assert_eq!(second.cache_misses, 0);

    std::fs::write(
        fixture.path().join("pkg/models.py"),
        r#"
class User:
    def __init__(self, user_id, email):
        self.user_id = user_id
        self.email = email

def model_version():
    return "changed"
"#,
    )
    .expect("modify model");
    let third =
        index_project(fixture.path().to_str().unwrap(), None, false, true).expect("third index");

    assert_eq!(third.scanned_files, second.scanned_files);
    assert!(third.cache_hits > 0);
    assert_eq!(third.cache_misses, 1);
}

#[test]
fn index_project_rebuilds_corrupt_cache_file() {
    let fixture = fixture();
    let first =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("first index");
    assert!(first.cache_misses > 0);

    let python_file = first
        .languages
        .iter()
        .find(|summary| summary.language == Language::Python)
        .expect("python summary");
    let index_path = fixture.path().join(&python_file.index_file);
    let shard: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(index_path).expect("language index"))
            .expect("language shard json");
    let cache_file = shard["files"][0]["cache_file"]
        .as_str()
        .expect("cache file path");
    std::fs::write(fixture.path().join(cache_file), "{ definitely not json").expect("corrupt");

    let rebuilt =
        index_project(fixture.path().to_str().unwrap(), None, false, true).expect("rebuild index");

    assert_eq!(rebuilt.scanned_files, first.scanned_files);
    assert_eq!(rebuilt.cache_misses, 1);
}

#[test]
fn cli_index_project_generates_manifest_json() {
    let fixture = fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rustrank"))
        .args([
            "index-project",
            "--repo-path",
            fixture.path().to_str().unwrap(),
            "--force-rebuild",
            "--clean-stale",
        ])
        .output()
        .expect("run index cli");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let summary: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cli json response");
    assert!(summary["indexed_files"].as_u64().unwrap() > 0);
    assert!(
        summary["project_manifest"]
            .as_str()
            .unwrap()
            .ends_with(".rustrank/index/v1/project_manifest.json")
    );
    assert!(
        fixture
            .path()
            .join(".rustrank/index/v1/project_manifest.json")
            .exists()
    );
}

#[test]
fn code_hotspots_detects_connected_modules() {
    let fixture = fixture();
    let rows = code_hotspots(fixture.path().to_str().unwrap(), 5, 1).expect("hotspots");

    assert!(rows.iter().any(|row| row.module.contains("pkg.core")));
}

#[test]
fn trace_data_flow_finds_identifier() {
    let fixture = fixture();
    let rows =
        trace_data_flow(fixture.path().to_str().unwrap(), "user_id", true, true).expect("flow");

    assert!(rows.iter().any(|row| row.file.ends_with("pkg/api.py")));
}

#[test]
fn trace_feature_impl_groups_layers() {
    let fixture = fixture();
    let rows = trace_feature_impl(fixture.path().to_str().unwrap(), &["login", "authenticate"])
        .expect("feature");

    assert!(rows.iter().any(|row| row.layer == "api"));
}

#[test]
fn trace_dep_impact_finds_dependents() {
    let fixture = fixture();
    let rows = trace_dep_impact(fixture.path().to_str().unwrap(), "pkg.core").expect("impact");

    assert!(rows.iter().any(|row| row.file.ends_with("pkg/api.py")));
}

#[test]
fn trace_dep_impact_finds_supported_language_dependents() {
    let fixture = fixture();
    let rust_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "crate.service").expect("rust impact");
    let csharp_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "App.Services").expect("csharp impact");
    let ts_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "./logger").expect("ts impact");
    let js_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "./format").expect("js impact");

    assert!(rust_rows.iter().any(|row| row.file.ends_with("src/lib.rs")));
    assert!(
        csharp_rows
            .iter()
            .any(|row| row.file.ends_with("app/Controller.cs"))
    );
    assert!(ts_rows.iter().any(|row| row.file.ends_with("web/auth.ts")));
    assert!(js_rows.iter().any(|row| row.file.ends_with("web/auth.js")));
}

#[test]
fn trace_dep_impact_accepts_canonical_resolved_modules() {
    let fixture = fixture();
    let py_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "pkg.models").expect("python impact");
    let rust_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "src.service").expect("rust impact");
    let csharp_rows = trace_dep_impact(fixture.path().to_str().unwrap(), "app.AuthService")
        .expect("csharp impact");
    let ts_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "web.logger").expect("ts impact");
    let js_rows =
        trace_dep_impact(fixture.path().to_str().unwrap(), "web.format").expect("js impact");

    assert!(
        py_rows
            .iter()
            .any(|row| row.file.ends_with("pkg/relative.py"))
    );
    assert!(rust_rows.iter().any(|row| row.file.ends_with("src/lib.rs")));
    assert!(
        csharp_rows
            .iter()
            .any(|row| row.file.ends_with("app/Controller.cs"))
    );
    assert!(ts_rows.iter().any(|row| row.file.ends_with("web/auth.ts")));
    assert!(js_rows.iter().any(|row| row.file.ends_with("web/auth.js")));
}

#[test]
fn error_patterns_detects_error_handling() {
    let fixture = fixture();
    let rows = error_patterns(fixture.path().to_str().unwrap(), true, false, None).expect("errors");

    assert!(rows.iter().any(|row| row.pattern == "raise"));
}

#[test]
fn error_patterns_can_include_git_evolution() {
    let fixture = fixture();
    commit_fixture(fixture.path());
    let rows =
        error_patterns(fixture.path().to_str().unwrap(), true, true, Some(36500)).expect("errors");

    assert!(rows.iter().any(|row| row.kind == "error_pattern_evolution"));
}

#[test]
fn perf_bottleneck_detects_sleep_in_loop() {
    let fixture = fixture();
    let rows = perf_bottleneck(fixture.path().to_str().unwrap(), &["sleep"], true).expect("perf");

    assert!(rows.iter().any(|row| row.pattern.contains("sleep")));
}

#[test]
fn exec_paths_finds_branches() {
    let fixture = fixture();
    let rows = exec_paths(fixture.path().to_str().unwrap(), "login", 4, true).expect("paths");

    assert!(rows.iter().any(|row| row.kind == "branch"));
}

#[test]
fn exec_paths_finds_supported_language_branches() {
    let fixture = fixture();
    let rust_rows =
        exec_paths(fixture.path().to_str().unwrap(), "login_user", 4, true).expect("rust paths");
    let csharp_rows =
        exec_paths(fixture.path().to_str().unwrap(), "Login", 4, true).expect("csharp paths");
    let ts_rows =
        exec_paths(fixture.path().to_str().unwrap(), "loginUser", 4, true).expect("ts paths");
    let js_rows =
        exec_paths(fixture.path().to_str().unwrap(), "loginBrowser", 4, true).expect("js paths");

    assert!(rust_rows.iter().any(|row| row.kind == "branch"));
    assert!(csharp_rows.iter().any(|row| row.kind == "branch"));
    assert!(ts_rows.iter().any(|row| row.kind == "branch"));
    assert!(js_rows.iter().any(|row| row.kind == "branch"));
}

#[test]
fn config_round_trips_json_values() {
    let fixture = fixture();
    let empty = get_config(fixture.path().to_str().unwrap()).expect("get");
    assert!(empty.is_null() || empty.as_object().is_some_and(|obj| obj.is_empty()));

    let updated = set_config(
        fixture.path().to_str().unwrap(),
        "threshold",
        serde_json::json!(3),
    )
    .expect("set");
    assert_eq!(updated["threshold"], 3);

    let got = get_config(fixture.path().to_str().unwrap()).expect("get updated");
    assert_eq!(got["threshold"], 3);
}

fn commit_fixture(root: &std::path::Path) {
    let repo = git2::Repository::init(root).expect("git init");
    let mut index = repo.index().expect("index");
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .expect("add all");
    let tree_id = index.write_tree().expect("write tree");
    let tree = repo.find_tree(tree_id).expect("tree");
    let signature =
        git2::Signature::now("RustRank Tests", "rustrank@example.invalid").expect("signature");
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "initial fixture",
        &tree,
        &[],
    )
    .expect("commit");
}
