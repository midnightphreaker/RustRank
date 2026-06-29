use rustrank::context::{Context, DefKind, Language, supported_source_files};
use rustrank::embeddings::EmbeddingOptions;
use rustrank::index::index_project_with_embeddings;
use rustrank::tools::ALL_TOOLS;
use rustrank::tools::agent::{
    detect_changes, impact, query, read_current_resource, resource_templates, resources,
    symbol_context,
};
use rustrank::tools::analysis::{error_patterns, exec_paths, perf_bottleneck};
use rustrank::tools::code_rank::{code_hotspots, coderank_analysis};
use rustrank::tools::config::{get_config, set_config};
use rustrank::tools::index::index_project;
use rustrank::tools::search::{api_usage, contextual_search, smart_code_search};
use rustrank::tools::trace::{trace_data_flow, trace_dep_impact, trace_feature_impl};

mod fixtures;

use fixtures::fixture;

#[test]
fn router_registers_agent_facing_tools() {
    assert_eq!(ALL_TOOLS.len(), 19);
    assert!(ALL_TOOLS.contains(&"exec_paths"));
    assert!(ALL_TOOLS.contains(&"execute_paths"));
    assert!(ALL_TOOLS.contains(&"index_project"));
    assert!(ALL_TOOLS.contains(&"context"));
    assert!(ALL_TOOLS.contains(&"impact"));
    assert!(ALL_TOOLS.contains(&"detect_changes"));
    assert!(ALL_TOOLS.contains(&"query"));
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
    assert_eq!(
        Language::from_path(std::path::Path::new("native/auth.c")),
        Some(Language::C)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("native/auth.h")),
        Some(Language::C)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("native/session.cpp")),
        Some(Language::Cpp)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("native/session.hpp")),
        Some(Language::Cpp)
    );
    assert_eq!(
        Language::from_path(std::path::Path::new("goapp/auth.go")),
        Some(Language::Go)
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
fn context_parse_all_recovers_from_malformed_python() {
    let fixture = fixture();
    std::fs::write(
        fixture.path().join("pkg/broken.py"),
        r#"
import os
from pkg.models import User

def broken():

class Recoverable:
    pass

async def later(value):
    return value
"#,
    )
    .expect("broken python");
    let ctx = Context::new(fixture.path().to_path_buf());

    let modules = ctx.parse_all().expect("parse all with broken python");
    let broken = modules
        .iter()
        .find(|module| module.path.ends_with("pkg/broken.py"))
        .expect("broken module indexed");

    assert!(
        broken
            .imports
            .iter()
            .any(|import| import.module == "pkg.models" && import.name.as_deref() == Some("User"))
    );
    assert!(broken.defs.iter().any(|def| def.name == "broken"));
    assert!(broken.defs.iter().any(|def| def.name == "Recoverable"));
    assert!(broken.defs.iter().any(|def| def.name == "later"));
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
    assert!(modules.iter().any(|module| module.language == Language::C));
    assert!(
        modules
            .iter()
            .any(|module| module.language == Language::Cpp)
    );
    assert!(modules.iter().any(|module| module.language == Language::Go));

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

    let c = modules
        .iter()
        .find(|module| module.path.ends_with("native/auth.c"))
        .expect("c module");
    assert!(c.imports.iter().any(|import| import.module == "auth.h"));
    assert!(c.defs.iter().any(|def| def.name == "native_login"));
    let c_header = modules
        .iter()
        .find(|module| module.path.ends_with("native/auth.h"))
        .expect("c header module");
    assert!(
        c_header
            .defs
            .iter()
            .any(|def| def.name == "native_login" && def.kind == DefKind::Func)
    );
    assert!(
        c_header
            .defs
            .iter()
            .any(|def| def.name == "XML_SetEncoding" && def.kind == DefKind::Func)
    );

    let cpp = modules
        .iter()
        .find(|module| module.path.ends_with("native/session.cpp"))
        .expect("cpp module");
    assert!(
        cpp.imports
            .iter()
            .any(|import| import.module == "session.hpp")
    );
    assert!(cpp.defs.iter().any(|def| def.name == "format_user"));
    let cpp_header = modules
        .iter()
        .find(|module| module.path.ends_with("native/session.hpp"))
        .expect("cpp header module");
    assert!(
        cpp_header
            .defs
            .iter()
            .any(|def| def.name == "DynamicArgList" && def.kind == DefKind::Class),
        "{:?}",
        cpp_header.defs
    );
    assert_eq!(
        cpp_header
            .defs
            .iter()
            .filter(|def| def.name == "DynamicArgList" && def.kind == DefKind::Class)
            .count(),
        1,
        "{:?}",
        cpp_header.defs
    );
    assert!(
        !cpp_header
            .defs
            .iter()
            .any(|def| matches!(def.name.as_str(), "FMT_EXPORT" | "namespace")),
        "{:?}",
        cpp_header.defs
    );

    let go = modules
        .iter()
        .find(|module| module.path.ends_with("goapp/auth.go"))
        .expect("go module");
    assert!(
        go.imports
            .iter()
            .any(|import| import.module == "fmt" && import.line == 5)
    );
    assert!(go.imports.iter().any(|import| import.module
        == "github.com/gin-gonic/gin/internal/fs"
        && import.name.as_deref() == Some("filesystem")
        && import.line == 6));
    assert!(go.defs.iter().any(|def| def.name == "GoUser"));
    assert!(
        go.defs
            .iter()
            .any(|def| def.name == "HandlerFunc" && def.kind == DefKind::Func),
        "{:?}",
        go.defs
    );
    assert!(go.defs.iter().any(|def| def.name == "LoginUser"));
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
fn supported_source_files_honors_default_and_configured_excludes() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.path().join(".venv")).expect("venv dir");
    std::fs::write(
        fixture.path().join(".venv/probe.py"),
        "def should_not_index_venv(): pass",
    )
    .expect("venv source");
    std::fs::create_dir_all(fixture.path().join("generated")).expect("generated dir");
    std::fs::write(
        fixture.path().join("generated/probe.py"),
        "def should_not_index_generated(): pass",
    )
    .expect("generated source");
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"excludes":{"paths":["generated/**"]}}"#,
    )
    .expect("config");

    let files = supported_source_files(fixture.path()).expect("source files");
    let paths = files
        .iter()
        .map(|(path, _)| path.strip_prefix(fixture.path()).unwrap().to_string_lossy())
        .collect::<Vec<_>>();

    assert!(!paths.iter().any(|path| path.contains(".venv")));
    assert!(!paths.iter().any(|path| path.contains("generated")));
    assert!(paths.iter().any(|path| path == "pkg/core.py"));
}

#[test]
fn h_headers_default_to_c_without_language_override() {
    let fixture = fixture();

    let files = supported_source_files(fixture.path()).expect("source files");
    let header = files
        .iter()
        .find(|(path, _)| path.ends_with("native/auth.h"))
        .expect("c header");
    let ctx = Context::new(fixture.path().to_path_buf());
    let parsed = ctx
        .get_or_parse("native/auth.h".to_string())
        .expect("parse header");

    assert_eq!(header.1, Language::C);
    assert_eq!(parsed.language, Language::C);
}

#[test]
fn language_overrides_classify_matching_h_headers_as_cpp() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.path().join("include/cpp")).expect("include dir");
    std::fs::write(
        fixture.path().join("include/cpp/widget.h"),
        r#"
#pragma once

class Widget {
public:
    int id() const;
};
"#,
    )
    .expect("cpp header");
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"languages":{"overrides":[{"paths":["include/cpp/**/*.h"],"language":"cpp"}]}}"#,
    )
    .expect("config");

    let files = supported_source_files(fixture.path()).expect("source files");
    let overridden = files
        .iter()
        .find(|(path, _)| path.ends_with("include/cpp/widget.h"))
        .expect("overridden header");
    let ordinary = files
        .iter()
        .find(|(path, _)| path.ends_with("native/auth.h"))
        .expect("ordinary header");

    assert_eq!(overridden.1, Language::Cpp);
    assert_eq!(ordinary.1, Language::C);
}

#[test]
fn language_overrides_keep_direct_and_bulk_parsing_consistent() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.path().join("include/cpp")).expect("include dir");
    std::fs::write(
        fixture.path().join("include/cpp/widget.h"),
        r#"
#pragma once

class Widget {
public:
    int id() const;
};
"#,
    )
    .expect("cpp header");
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"languages":{"overrides":[{"paths":["include/cpp/**/*.h"],"language":"cpp"}]}}"#,
    )
    .expect("config");

    let ctx = Context::new(fixture.path().to_path_buf());
    let direct = ctx
        .get_or_parse("include/cpp/widget.h".to_string())
        .expect("direct parse");
    let bulk = ctx
        .parse_all()
        .expect("bulk parse")
        .into_iter()
        .find(|module| module.path.ends_with("include/cpp/widget.h"))
        .expect("bulk module");

    assert_eq!(direct.language, Language::Cpp);
    assert_eq!(bulk.language, Language::Cpp);
}

#[test]
fn index_project_writes_overridden_h_headers_to_cpp_shard() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.path().join("include/cpp")).expect("include dir");
    std::fs::write(
        fixture.path().join("include/cpp/widget.h"),
        r#"
#pragma once

class Widget {
public:
    int id() const;
};
"#,
    )
    .expect("cpp header");
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"languages":{"enabled":["c","cpp"],"overrides":[{"paths":["include/cpp/**/*.h"],"language":"cpp"}]}}"#,
    )
    .expect("config");

    let summary =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");
    let cpp_summary = summary
        .languages
        .iter()
        .find(|language| language.language == Language::Cpp)
        .expect("cpp summary");
    let c_summary = summary
        .languages
        .iter()
        .find(|language| language.language == Language::C)
        .expect("c summary");
    let cpp_index: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.path().join(&cpp_summary.index_file)).expect("cpp index"),
    )
    .expect("cpp index json");
    let c_index: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixture.path().join(&c_summary.index_file)).expect("c index"),
    )
    .expect("c index json");

    assert!(
        cpp_index["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "include/cpp/widget.h"),
        "{cpp_index}"
    );
    assert!(
        !c_index["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "include/cpp/widget.h"),
        "{c_index}"
    );
}

#[test]
fn index_project_warns_for_invalid_language_override_name() {
    let fixture = fixture();
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"languages":{"overrides":[{"paths":["native/**/*.h"],"language":"objective-c"}]}}"#,
    )
    .expect("config");

    let summary =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");

    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("ignored unsupported language override")),
        "{:?}",
        summary.warnings
    );
}

#[test]
fn language_override_invalid_glob_returns_validation_error() {
    let fixture = fixture();
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"languages":{"overrides":[{"paths":["include/**/[broken"],"language":"cpp"}]}}"#,
    )
    .expect("config");

    let err = supported_source_files(fixture.path()).expect_err("invalid glob");

    assert!(err.to_string().contains("error parsing glob"), "{err}");
}

#[test]
fn language_overrides_respect_enabled_language_filtering() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.path().join("include/cpp")).expect("include dir");
    std::fs::write(
        fixture.path().join("include/cpp/widget.h"),
        r#"
#pragma once

class Widget {
public:
    int id() const;
};
"#,
    )
    .expect("cpp header");
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"languages":{"enabled":["c"],"overrides":[{"paths":["include/cpp/**/*.h"],"language":"cpp"}]}}"#,
    )
    .expect("config");

    let modules = Context::new(fixture.path().to_path_buf())
        .parse_all()
        .expect("parse filtered");

    assert!(
        modules
            .iter()
            .any(|module| module.path.ends_with("native/auth.h") && module.language == Language::C)
    );
    assert!(
        !modules
            .iter()
            .any(|module| module.path.ends_with("include/cpp/widget.h")),
        "{modules:?}"
    );
}

#[test]
fn contextual_search_honors_configured_excludes_for_filtered_searches() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.path().join(".venv")).expect("venv dir");
    std::fs::write(
        fixture.path().join(".venv/probe.py"),
        "def should_not_find_venv_probe(): pass",
    )
    .expect("venv source");
    std::fs::create_dir_all(fixture.path().join("generated")).expect("generated dir");
    std::fs::write(
        fixture.path().join("generated/probe.py"),
        "def should_not_find_generated_probe(): pass",
    )
    .expect("generated source");
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"excludes":{"paths":["generated/**"]}}"#,
    )
    .expect("config");

    let rows = contextual_search(
        fixture.path().to_str().unwrap(),
        "should_not_find",
        Some("py"),
        false,
        0,
    )
    .expect("search");

    assert!(rows.is_empty());
}

#[test]
fn index_project_honors_configured_excludes() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.path().join(".venv")).expect("venv dir");
    std::fs::write(
        fixture.path().join(".venv/probe.py"),
        "def should_not_index_venv(): pass",
    )
    .expect("venv source");
    std::fs::create_dir_all(fixture.path().join("generated")).expect("generated dir");
    std::fs::write(
        fixture.path().join("generated/probe.py"),
        "def should_not_index_generated(): pass",
    )
    .expect("generated source");
    std::fs::write(
        fixture.path().join(".rustrank_config.json"),
        r#"{"languages":{"enabled":["python"]},"excludes":{"paths":["generated/**"]}}"#,
    )
    .expect("config");

    let response =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");
    let manifest = std::fs::read_to_string(
        fixture
            .path()
            .join(".rustrank/index/v1/project_manifest.json"),
    )
    .expect("manifest");

    assert_eq!(response.indexed_files, 4);
    assert!(!manifest.contains(".venv/probe.py"));
    assert!(!manifest.contains("generated/probe.py"));
    assert!(manifest.contains("pkg/core.py"));
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
fn smart_code_search_scores_c_results() {
    let fixture = fixture();
    let rows = smart_code_search(fixture.path().to_str().unwrap(), "native_login", 1, 10)
        .expect("smart c");

    assert!(
        rows.iter()
            .any(|row| row.file.ends_with("native/auth.c") && row.score > 0.0),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.file.ends_with("native/auth.h") && row.score > 0.0),
        "{rows:?}"
    );
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
    assert!(
        rows.iter()
            .any(|row| row.module == "native.auth" && row.imports + row.depth > 0),
        "{rows:?}"
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
    assert!(
        !modules
            .iter()
            .any(|module| matches!(module.language, Language::C | Language::Cpp | Language::Go))
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
    assert!(manifest["languages"].as_array().unwrap().len() >= 8);
    for language in ["c", "cpp", "go"] {
        assert!(
            summary
                .languages
                .iter()
                .any(|row| row.language.config_name() == language && row.files > 0),
            "{:?}",
            summary.languages
        );
        assert!(
            manifest["languages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["language"] == language),
            "{:?}",
            manifest["languages"]
        );
    }
    assert_eq!(
        manifest["nodes"].as_array().unwrap()[0]["schema"],
        "graph_node"
    );
    assert!(
        manifest["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["kind"] == "symbol" && node["name"] == "authenticate")
    );
    assert!(
        manifest["graph_edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["kind"] == "DEFINES")
    );
    assert!(
        manifest["graph_edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["kind"] == "CALLS")
    );
    assert!(
        manifest["processes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|process| process["name"] == "authenticate"
                && process["call_chain"]
                    .as_array()
                    .is_some_and(|chain| chain.iter().any(|step| step["symbol"] == "login")))
    );
    assert!(
        manifest["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |edge| edge["source_module"] == "pkg.core" && edge["target_module"] == "pkg.models"
            )
    );
    assert!(manifest["freshness"]["indexed_head"].is_string());
    assert!(manifest["freshness"]["current_head"].is_string());

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
    assert!(agents.contains("Agent-facing tools"));
    assert!(agents.contains("rustrank://repo/current/context"));
    assert!(agents.contains("rustrank://repo/current/processes"));
    assert!(
        fixture
            .path()
            .join(".rustrank/skills/exploring.md")
            .exists()
    );
}

#[test]
fn resource_helpers_expose_current_repo_context() {
    let fixture = fixture();
    index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");
    let old_cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(fixture.path()).expect("set cwd");

    let listed = resources().expect("resources");
    let templates = resource_templates();
    let context = read_current_resource("rustrank://repo/current/context").expect("context");
    let modules = read_current_resource("rustrank://repo/current/modules").expect("modules");
    let processes = read_current_resource("rustrank://repo/current/processes").expect("processes");
    let module = read_current_resource("rustrank://repo/current/module/pkg.core").expect("module");
    let process =
        read_current_resource("rustrank://repo/current/process/authenticate").expect("process");
    let unknown = read_current_resource("rustrank://repo/current/module/missing");
    let unknown_process = read_current_resource("rustrank://repo/current/process/missing");
    let unknown_uri = read_current_resource("rustrank://repo/current/unknown");

    std::env::set_current_dir(old_cwd).expect("restore cwd");

    assert!(
        listed
            .iter()
            .any(|resource| resource.uri == "rustrank://repo/current/context")
    );
    assert!(
        listed
            .iter()
            .any(|resource| resource.uri == "rustrank://repo/current/processes")
    );
    assert!(
        templates
            .iter()
            .any(|template| template.uri_template == "rustrank://repo/current/module/{name}")
    );
    assert!(
        templates
            .iter()
            .any(|template| template.uri_template == "rustrank://repo/current/process/{name}")
    );
    assert!(context.contains("RustRank repository context"));
    assert!(modules.contains("RustRank modules"));
    assert!(processes.contains("RustRank processes"));
    assert!(processes.contains("authenticate"));
    assert!(module.contains("authenticate"));
    assert!(process.contains("Process `authenticate`"));
    assert!(process.contains("Call chain"));
    assert!(process.contains("login"));
    assert!(unknown.is_err());
    assert!(
        unknown_process
            .expect_err("unknown process")
            .to_string()
            .contains("process not found: missing")
    );
    assert!(unknown_uri.is_err());
}

#[test]
fn context_impact_change_and_query_tools_return_agent_graph_context() {
    let fixture = fixture();
    index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(fixture.path())
        .output()
        .expect("git init");
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(fixture.path())
        .output()
        .expect("git add");
    std::process::Command::new("git")
        .args(["commit", "-m", "fixture"])
        .env("GIT_AUTHOR_NAME", "RustRank Test")
        .env("GIT_AUTHOR_EMAIL", "rustrank@example.test")
        .env("GIT_COMMITTER_NAME", "RustRank Test")
        .env("GIT_COMMITTER_EMAIL", "rustrank@example.test")
        .current_dir(fixture.path())
        .output()
        .expect("git commit");
    std::fs::write(
        fixture.path().join("pkg/core.py"),
        r#"
import time
from pkg.models import User

def authenticate(user_id, email):
    return User(user_id, email)
"#,
    )
    .expect("modify core");

    let context =
        symbol_context(fixture.path().to_str().unwrap(), "authenticate").expect("context");
    let impact = impact(fixture.path().to_str().unwrap(), "authenticate", 3).expect("impact");
    let changes = detect_changes(fixture.path().to_str().unwrap()).expect("changes");
    let results = query(fixture.path().to_str().unwrap(), "login authenticate", 5).expect("query");

    assert_eq!(context.symbol, "authenticate");
    assert!(context.defining_file.ends_with("pkg/core.py"));
    assert!(impact.edges.iter().any(|edge| edge.confidence == "high"));
    assert!(
        changes
            .changed_symbols
            .iter()
            .any(|symbol| symbol.name == "authenticate")
    );
    assert!(results.iter().any(|row| row.file.ends_with("pkg/core.py")));
    assert!(results.iter().any(|row| row.process.as_deref().is_some()));
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
fn index_project_rebuilds_non_utf8_cache_file() {
    let fixture = fixture();
    let first =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("first index");

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
    std::fs::write(fixture.path().join(cache_file), [0xff, 0xfe, 0x00]).expect("corrupt cache");

    let rebuilt =
        index_project(fixture.path().to_str().unwrap(), None, false, true).expect("rebuild index");

    assert_eq!(rebuilt.scanned_files, first.scanned_files);
    assert_eq!(rebuilt.cache_misses, 1);
    assert!(
        rebuilt
            .warnings
            .iter()
            .any(|warning| warning.contains("ignored unreadable cache file")),
        "{:?}",
        rebuilt.warnings
    );
}

#[test]
fn index_project_skips_non_utf8_source_with_warning() {
    let fixture = fixture();
    std::fs::write(
        fixture.path().join("pkg/binary.py"),
        [0xff, 0xfe, 0x00, 0x61],
    )
    .expect("binary source");

    let summary =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");

    assert_eq!(summary.indexed_files + 1, summary.scanned_files);
    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("skipped non-UTF-8 source file `pkg/binary.py`")),
        "{:?}",
        summary.warnings
    );
    let manifest_text = std::fs::read_to_string(
        fixture
            .path()
            .join(".rustrank/index/v1/project_manifest.json"),
    )
    .expect("manifest");
    assert!(!manifest_text.contains("pkg/binary.py"));
}

#[test]
fn index_project_warns_and_skips_non_utf8_agents_md_update() {
    let fixture = fixture();
    std::fs::write(fixture.path().join("AGENTS.md"), [0xff, 0xfe, 0x00]).expect("agents file");

    let summary =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");

    assert!(
        summary
            .warnings
            .iter()
            .any(|warning| warning.contains("skipped AGENTS.md update")),
        "{:?}",
        summary.warnings
    );
    assert_eq!(
        std::fs::read(fixture.path().join("AGENTS.md")).expect("agents bytes"),
        vec![0xff, 0xfe, 0x00]
    );
}

#[test]
fn index_project_accepts_partial_tree_sitter_parse() {
    let fixture = fixture();
    std::fs::write(
        fixture.path().join("web/partial.ts"),
        "export function halfWritten(\n",
    )
    .expect("partial source");

    let summary =
        index_project(fixture.path().to_str().unwrap(), None, true, true).expect("index project");

    assert_eq!(summary.scanned_files, summary.indexed_files);
    assert!(
        summary
            .languages
            .iter()
            .any(|language| language.language == Language::TypeScript && language.files >= 2)
    );
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
fn cli_index_project_accepts_embedding_flags_without_exposing_api_key() {
    let fixture = fixture();
    let fake_key = "rrk-test-secret-do-not-log";
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rustrank"))
        .args([
            "index-project",
            "--repo-path",
            fixture.path().to_str().unwrap(),
            "--force-rebuild",
            "--clean-stale",
            "--embeddings",
            "--embedding-base-url",
            "http://127.0.0.1:9/v1",
            "--embedding-model",
            "text-embedding-test",
            "--embedding-dims",
            "3",
            "--embedding-api-key",
            fake_key,
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(fake_key));
    assert!(!stderr.contains(fake_key));
}

#[test]
fn cli_help_prints_usage_successfully() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rustrank"))
        .arg("--help")
        .output()
        .expect("run help");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "{stdout}");
    assert!(stdout.contains("index-project"), "{stdout}");
    assert!(stdout.contains("--list-tools"), "{stdout}");
}

#[test]
fn cli_index_project_help_prints_usage_successfully() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rustrank"))
        .args(["index-project", "--help"])
        .output()
        .expect("run index help");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "{stdout}");
    assert!(stdout.contains("--repo-path <PATH>"), "{stdout}");
    assert!(stdout.contains("--force-rebuild"), "{stdout}");
    assert!(stdout.contains("--clean-stale"), "{stdout}");
}

#[test]
fn cli_index_project_invalid_args_return_structured_json() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rustrank"))
        .arg("index-project")
        .output()
        .expect("run invalid index cli");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).expect("error json");
    assert_eq!(error["error"], true);
    assert_eq!(error["code"], "INVALID_ARGUMENTS");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("--repo-path")),
        "{error}"
    );
    assert!(
        error["suggestion"]
            .as_str()
            .is_some_and(|suggestion| suggestion.contains("rustrank index-project")),
        "{error}"
    );
}

#[test]
fn embedding_options_debug_redacts_api_key() {
    let options = embedding_options("http://127.0.0.1:9/v1", Some("secret-test-key"));

    let rendered = format!("{options:?}");

    assert!(!rendered.contains("secret-test-key"));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn embedding_index_reuses_cached_vectors() {
    let fixture = fixture();
    let server = MockEmbeddingServer::start(MockEmbeddingBehavior::Fixed(vec![0.1, 0.2, 0.3]));
    let options = embedding_options(&server.base_url, None);

    let first = index_project_with_embeddings(
        fixture.path().to_str().unwrap(),
        Some(vec!["python".to_string()]),
        true,
        true,
        options.clone(),
    )
    .expect("first embedding index");
    let after_first = server.request_count();
    let second = index_project_with_embeddings(
        fixture.path().to_str().unwrap(),
        Some(vec!["python".to_string()]),
        false,
        true,
        options,
    )
    .expect("second embedding index");

    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    assert!(after_first > 0);
    assert_eq!(server.request_count(), after_first);
    assert!(
        fixture
            .path()
            .join(".rustrank/index/v1/embeddings")
            .exists()
    );
}

#[test]
fn embedding_index_rebuilds_non_utf8_cache_file() {
    let fixture = fixture();
    let server = MockEmbeddingServer::start(MockEmbeddingBehavior::Fixed(vec![0.1, 0.2, 0.3]));
    let options = embedding_options(&server.base_url, None);
    index_project_with_embeddings(
        fixture.path().to_str().unwrap(),
        Some(vec!["python".to_string()]),
        true,
        true,
        options.clone(),
    )
    .expect("first embedding index");
    let after_first = server.request_count();
    let cache_dir = fixture.path().join(".rustrank/index/v1/embeddings");
    let cache_file = std::fs::read_dir(&cache_dir)
        .expect("embedding cache dir")
        .find_map(|entry| {
            let path = entry.expect("entry").path();
            (path.extension().is_some_and(|ext| ext == "json")).then_some(path)
        })
        .expect("embedding cache file");
    std::fs::write(&cache_file, [0xff, 0xfe, 0x00]).expect("corrupt embedding cache");

    let rebuilt = index_project_with_embeddings(
        fixture.path().to_str().unwrap(),
        Some(vec!["python".to_string()]),
        false,
        true,
        options,
    )
    .expect("rebuild embedding index");

    assert!(server.request_count() > after_first);
    assert!(
        rebuilt
            .warnings
            .iter()
            .any(|warning| warning.contains("embedding cache read failed")),
        "{:?}",
        rebuilt.warnings
    );
}

#[test]
fn embedding_index_warns_on_dimension_mismatch() {
    let fixture = fixture();
    let server = MockEmbeddingServer::start(MockEmbeddingBehavior::Fixed(vec![0.1, 0.2]));

    let response = index_project_with_embeddings(
        fixture.path().to_str().unwrap(),
        Some(vec!["python".to_string()]),
        true,
        true,
        embedding_options(&server.base_url, None),
    )
    .expect("embedding index");

    assert!(
        response
            .warnings
            .iter()
            .any(|warning| warning.contains("embedding dimension mismatch")),
        "{:?}",
        response.warnings
    );
}

#[test]
fn embedding_index_omits_authorization_without_api_key() {
    let fixture = fixture();
    let server = MockEmbeddingServer::start(MockEmbeddingBehavior::Fixed(vec![0.1, 0.2, 0.3]));

    index_project_with_embeddings(
        fixture.path().to_str().unwrap(),
        Some(vec!["python".to_string()]),
        true,
        true,
        embedding_options(&server.base_url, None),
    )
    .expect("embedding index");

    assert!(server.request_count() > 0);
    assert!(
        server.authorization_headers().iter().all(Option::is_none),
        "{:?}",
        server.authorization_headers()
    );
}

#[test]
fn query_uses_cached_embeddings_for_semantic_ranking() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("pkg")).expect("pkg dir");
    std::fs::write(
        dir.path().join("pkg/billing.py"),
        "def billing_invoice():\n    return 'invoice ledger'\n",
    )
    .expect("billing");
    std::fs::write(
        dir.path().join("pkg/auth.py"),
        "def auth_token():\n    return 'session token'\n",
    )
    .expect("auth");
    let server = MockEmbeddingServer::start(MockEmbeddingBehavior::ByInput);
    set_config(
        dir.path().to_str().unwrap(),
        "embeddings.enabled",
        serde_json::json!(true),
    )
    .expect("enable embeddings");
    set_config(
        dir.path().to_str().unwrap(),
        "embeddings.base_url",
        serde_json::json!(server.base_url),
    )
    .expect("base url");
    set_config(
        dir.path().to_str().unwrap(),
        "embeddings.model",
        serde_json::json!("text-embedding-test"),
    )
    .expect("model");
    set_config(
        dir.path().to_str().unwrap(),
        "embeddings.dimensions",
        serde_json::json!(3),
    )
    .expect("dimensions");
    index_project_with_embeddings(
        dir.path().to_str().unwrap(),
        Some(vec!["python".to_string()]),
        true,
        true,
        EmbeddingOptions {
            enabled: Some(true),
            base_url: None,
            model: None,
            dimensions: None,
            api_key: None,
        },
    )
    .expect("index embeddings");

    let rows = query(dir.path().to_str().unwrap(), "payments", 5).expect("semantic query");

    assert!(
        rows.first()
            .is_some_and(|row| row.file.ends_with("pkg/billing.py")),
        "{rows:?}"
    );
    assert!(
        rows.first()
            .is_some_and(|row| row.match_reasons.iter().any(|reason| reason == "semantic")),
        "{rows:?}"
    );
}

#[test]
fn code_hotspots_detects_connected_modules() {
    let fixture = fixture();
    let rows = code_hotspots(fixture.path().to_str().unwrap(), 50, 1).expect("hotspots");

    assert!(rows.iter().any(|row| row.module.contains("pkg.core")));
    assert!(
        rows.iter().any(|row| row.module == "native.auth"),
        "{rows:?}"
    );
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

    let updated = set_config(
        fixture.path().to_str().unwrap(),
        "embeddings.enabled",
        serde_json::json!(true),
    )
    .expect("set nested");
    assert_eq!(updated["embeddings"]["enabled"], true);

    let updated = set_config(
        fixture.path().to_str().unwrap(),
        "embeddings.model",
        serde_json::json!("text-image-embedding"),
    )
    .expect("set nested model");
    assert_eq!(updated["embeddings"]["model"], "text-image-embedding");
}

#[derive(Clone)]
struct MockEmbeddingServer {
    base_url: String,
    requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    authorization_headers: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
}

enum MockEmbeddingBehavior {
    Fixed(Vec<f32>),
    ByInput,
}

impl MockEmbeddingServer {
    fn start(behavior: MockEmbeddingBehavior) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base_url = format!("http://{}/v1", listener.local_addr().expect("addr"));
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let authorization_headers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let thread_requests = requests.clone();
        let thread_authorization_headers = authorization_headers.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let request = read_http_request(&mut stream);
                thread_requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                thread_authorization_headers
                    .lock()
                    .expect("auth headers")
                    .push(authorization_header(&request));
                let vector = match &behavior {
                    MockEmbeddingBehavior::Fixed(vector) => vector.clone(),
                    MockEmbeddingBehavior::ByInput => vector_for_request(&request),
                };
                let body = serde_json::json!({
                    "data": [{ "embedding": vector }]
                })
                .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                use std::io::Write;
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        Self {
            base_url,
            requests,
            authorization_headers,
        }
    }

    fn request_count(&self) -> usize {
        self.requests.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn authorization_headers(&self) -> Vec<Option<String>> {
        self.authorization_headers
            .lock()
            .expect("auth headers")
            .clone()
    }
}

fn embedding_options(base_url: &str, api_key: Option<&str>) -> EmbeddingOptions {
    EmbeddingOptions {
        enabled: Some(true),
        base_url: Some(base_url.to_string()),
        model: Some("text-embedding-test".to_string()),
        dimensions: Some(3),
        api_key: api_key.map(ToOwned::to_owned),
    }
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;

    let mut buffer = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let request = String::from_utf8_lossy(&buffer);
        if let Some(header_end) = request.find("\r\n\r\n") {
            let headers = &request[..header_end];
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if buffer.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

fn authorization_header(request: &str) -> Option<String> {
    request.lines().find_map(|line| {
        line.strip_prefix("authorization: ")
            .or_else(|| line.strip_prefix("Authorization: "))
            .map(ToOwned::to_owned)
    })
}

fn vector_for_request(request: &str) -> Vec<f32> {
    let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
    let input = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("input")
                .and_then(|input| input.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    if input.contains("billing_invoice") || input.contains("payments") {
        vec![1.0, 0.0, 0.0]
    } else if input.contains("auth_token") {
        vec![0.0, 1.0, 0.0]
    } else {
        vec![0.0, 0.0, 1.0]
    }
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
