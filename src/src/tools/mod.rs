pub mod analysis;
pub mod code_rank;
pub mod config;
pub mod search;
pub mod trace;

use std::net::SocketAddr;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Deserialize;

pub const ALL_TOOLS: &[&str] = &[
    "contextual_search",
    "smart_code_search",
    "api_usage",
    "coderank_analysis",
    "code_hotspots",
    "trace_data_flow",
    "trace_feature_impl",
    "trace_dep_impact",
    "error_patterns",
    "perf_bottleneck",
    "exec_paths",
    "execute_paths",
    "get_config",
    "set_config",
];

#[derive(Debug, Clone)]
pub struct RustRankRouter {
    tool_router: ToolRouter<Self>,
}

impl RustRankRouter {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

impl Default for RustRankRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContextualSearchRequest {
    path: String,
    pattern: String,
    file_type: Option<String>,
    is_regex: bool,
    num_context_lines: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SmartCodeSearchRequest {
    repo_path: String,
    pattern: String,
    context_lines: usize,
    num_context_lines: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ApiUsageRequest {
    repo_path: String,
    api_name: String,
    max_examples: usize,
    group_by_pattern: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CodeRankRequest {
    repo_path: String,
    top_n: usize,
    module_prefix: Option<String>,
    external_modules: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HotspotRequest {
    repo_path: String,
    top_n: usize,
    min_connections: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DataFlowRequest {
    repo_path: String,
    identifier: String,
    include_transformations: bool,
    include_side_effects: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FeatureRequest {
    repo_path: String,
    feature_keywords: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DepImpactRequest {
    repo_path: String,
    target_module: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ErrorPatternsRequest {
    repo_path: String,
    include_antipatterns: bool,
    show_evolution: bool,
    days_back: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PerfRequest {
    repo_path: String,
    focus_areas: Vec<String>,
    include_utility: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExecPathsRequest {
    repo_path: String,
    function_name: String,
    max_depth: usize,
    include_call_contexts: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConfigPathRequest {
    repo_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetConfigRequest {
    repo_path: String,
    key: String,
    value: serde_json::Value,
}

#[tool_router]
impl RustRankRouter {
    #[tool(description = "Search repository files for a pattern with line context")]
    fn contextual_search(&self, Parameters(req): Parameters<ContextualSearchRequest>) -> String {
        json(search::contextual_search(
            &req.path,
            &req.pattern,
            req.file_type.as_deref(),
            req.is_regex,
            req.num_context_lines,
        ))
    }

    #[tool(description = "Search code and rank results by module importance")]
    fn smart_code_search(&self, Parameters(req): Parameters<SmartCodeSearchRequest>) -> String {
        json(search::smart_code_search(
            &req.repo_path,
            &req.pattern,
            req.context_lines,
            req.num_context_lines,
        ))
    }

    #[tool(description = "Find API usage examples grouped by usage pattern")]
    fn api_usage(&self, Parameters(req): Parameters<ApiUsageRequest>) -> String {
        json(search::api_usage(
            &req.repo_path,
            &req.api_name,
            req.max_examples,
            req.group_by_pattern,
        ))
    }

    #[tool(description = "Rank Python modules using import-graph PageRank")]
    fn coderank_analysis(&self, Parameters(req): Parameters<CodeRankRequest>) -> String {
        json(code_rank::coderank_analysis(
            &req.repo_path,
            req.top_n,
            req.module_prefix.as_deref(),
            req.external_modules,
        ))
    }

    #[tool(description = "Find modules that are important and frequently referenced")]
    fn code_hotspots(&self, Parameters(req): Parameters<HotspotRequest>) -> String {
        json(code_rank::code_hotspots(
            &req.repo_path,
            req.top_n,
            req.min_connections,
        ))
    }

    #[tool(description = "Trace occurrences and transformations of a data identifier")]
    fn trace_data_flow(&self, Parameters(req): Parameters<DataFlowRequest>) -> String {
        json(trace::trace_data_flow(
            &req.repo_path,
            &req.identifier,
            req.include_transformations,
            req.include_side_effects,
        ))
    }

    #[tool(description = "Map feature keywords across code layers")]
    fn trace_feature_impl(&self, Parameters(req): Parameters<FeatureRequest>) -> String {
        let keywords = req
            .feature_keywords
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        json(trace::trace_feature_impl(&req.repo_path, &keywords))
    }

    #[tool(description = "Find direct dependency impact for a target module")]
    fn trace_dep_impact(&self, Parameters(req): Parameters<DepImpactRequest>) -> String {
        json(trace::trace_dep_impact(&req.repo_path, &req.target_module))
    }

    #[tool(description = "Find error handling patterns and antipatterns")]
    fn error_patterns(&self, Parameters(req): Parameters<ErrorPatternsRequest>) -> String {
        json(analysis::error_patterns(
            &req.repo_path,
            req.include_antipatterns,
            req.show_evolution,
            req.days_back,
        ))
    }

    #[tool(description = "Detect simple performance bottleneck patterns")]
    fn perf_bottleneck(&self, Parameters(req): Parameters<PerfRequest>) -> String {
        let focus = req
            .focus_areas
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        json(analysis::perf_bottleneck(
            &req.repo_path,
            &focus,
            req.include_utility,
        ))
    }

    #[tool(description = "Trace branch and loop execution paths for a function")]
    fn exec_paths(&self, Parameters(req): Parameters<ExecPathsRequest>) -> String {
        json(analysis::exec_paths(
            &req.repo_path,
            &req.function_name,
            req.max_depth,
            req.include_call_contexts,
        ))
    }

    #[tool(description = "Trace branch and loop execution paths for a function")]
    fn execute_paths(&self, Parameters(req): Parameters<ExecPathsRequest>) -> String {
        json(analysis::execute_paths(
            &req.repo_path,
            &req.function_name,
            req.max_depth,
            req.include_call_contexts,
        ))
    }

    #[tool(description = "Read RustRank JSON configuration")]
    fn get_config(&self, Parameters(req): Parameters<ConfigPathRequest>) -> String {
        json(config::get_config(&req.repo_path))
    }

    #[tool(description = "Set a RustRank JSON configuration value")]
    fn set_config(&self, Parameters(req): Parameters<SetConfigRequest>) -> String {
        json(config::set_config(&req.repo_path, &req.key, req.value))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RustRankRouter {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("RustRank repository analysis tools")
    }
}

pub fn serve() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--list-tools") {
        println!("{}", ALL_TOOLS.join("\n"));
        return Ok(());
    }

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match transport_from_env() {
            Transport::StreamableHttp => serve_streamable_http().await,
            Transport::Stdio => serve_stdio().await,
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Stdio,
    StreamableHttp,
}

fn transport_from_env() -> Transport {
    let value = std::env::var("RUSTRANK_TRANSPORT")
        .or_else(|_| std::env::var("RUSTANK_TRANSPORT"))
        .ok();
    transport_from_value(value.as_deref())
}

fn transport_from_value(value: Option<&str>) -> Transport {
    match value.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value)
            if matches!(
                value.as_str(),
                "http" | "streamable_http" | "streamable-http"
            ) =>
        {
            Transport::StreamableHttp
        }
        _ => Transport::Stdio,
    }
}

async fn serve_stdio() -> anyhow::Result<()> {
    let service = RustRankRouter::new()
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

async fn serve_streamable_http() -> anyhow::Result<()> {
    let addr = std::env::var("RUSTRANK_LISTEN_ADDR")
        .or_else(|_| std::env::var("RUSTANK_LISTEN_ADDR"))
        .unwrap_or_else(|_| "127.0.0.1:63477".to_string());
    let addr: SocketAddr = addr.parse()?;
    let allowed_hosts = allowed_hosts_for(addr);
    let service: StreamableHttpService<RustRankRouter, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(RustRankRouter::new()),
            Default::default(),
            StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts),
        );
    let app = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!(
        "RustRank Streamable HTTP listening on http://{}/mcp",
        listener.local_addr()?
    );
    axum::serve(listener, app).await?;
    Ok(())
}

fn allowed_hosts_for(addr: SocketAddr) -> Vec<String> {
    let host = addr.ip().to_string();
    let host_port = format!("{}:{}", addr.ip(), addr.port());
    let mut hosts = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
        host,
        host_port,
    ];
    hosts.sort();
    hosts.dedup();
    hosts
}

fn json<T: serde::Serialize>(result: crate::Result<T>) -> String {
    match result {
        Ok(value) => serde_json::to_string(&value)
            .unwrap_or_else(|err| serde_json::json!({ "error": err.to_string() }).to_string()),
        Err(err) => serde_json::json!({ "error": err.to_string() }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_from_value_recognizes_http_aliases() {
        assert_eq!(
            transport_from_value(Some("streamable_http")),
            Transport::StreamableHttp
        );
        assert_eq!(
            transport_from_value(Some("http")),
            Transport::StreamableHttp
        );
        assert_eq!(
            transport_from_value(Some("streamable-http")),
            Transport::StreamableHttp
        );
    }

    #[test]
    fn transport_from_value_defaults_to_stdio() {
        assert_eq!(transport_from_value(None), Transport::Stdio);
        assert_eq!(transport_from_value(Some("stdio")), Transport::Stdio);
    }

    #[test]
    fn allowed_hosts_include_bound_loopback_authority() {
        let hosts = allowed_hosts_for("127.0.0.1:63477".parse().expect("addr"));

        assert!(hosts.iter().any(|host| host == "127.0.0.1"));
        assert!(hosts.iter().any(|host| host == "127.0.0.1:63477"));
    }
}
