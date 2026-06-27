pub mod agent;
pub mod analysis;
pub mod code_rank;
pub mod config;
pub mod index;
pub mod search;
pub mod trace;

use std::{future::Future, net::SocketAddr};

use crate::embeddings::EmbeddingOptions;
use axum::routing::get;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
        ServerInfo,
    },
    schemars,
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Deserialize;

const DEFAULT_HTTP_HOST: &str = "127.0.0.1";
const DEFAULT_HTTP_PORT: &str = "63477";
const DEFAULT_MCP_PATH: &str = "/mcp";
const HEALTH_PATH: &str = "/healthz";

pub const ALL_TOOLS: &[&str] = &[
    "index_project",
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
    "context",
    "impact",
    "detect_changes",
    "query",
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

#[derive(Deserialize, schemars::JsonSchema)]
struct IndexProjectRequest {
    repo_path: String,
    languages: Option<Vec<String>>,
    force_rebuild: bool,
    clean_stale: bool,
    #[serde(default)]
    embeddings: Option<bool>,
    #[serde(default)]
    embedding_base_url: Option<String>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    embedding_dims: Option<usize>,
    #[serde(default)]
    embedding_api_key: Option<String>,
}

impl std::fmt::Debug for IndexProjectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexProjectRequest")
            .field("repo_path", &self.repo_path)
            .field("languages", &self.languages)
            .field("force_rebuild", &self.force_rebuild)
            .field("clean_stale", &self.clean_stale)
            .field("embeddings", &self.embeddings)
            .field("embedding_base_url", &self.embedding_base_url)
            .field("embedding_model", &self.embedding_model)
            .field("embedding_dims", &self.embedding_dims)
            .field(
                "embedding_api_key",
                &self.embedding_api_key.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContextRequest {
    repo_path: String,
    symbol: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ImpactRequest {
    repo_path: String,
    target: String,
    #[serde(default = "default_impact_depth")]
    max_depth: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryRequest {
    repo_path: String,
    query: String,
    #[serde(default = "default_query_limit")]
    limit: usize,
}

#[tool_router]
impl RustRankRouter {
    #[tool(
        description = "Index a repository into persistent per-language caches and a project manifest"
    )]
    fn index_project(&self, Parameters(req): Parameters<IndexProjectRequest>) -> String {
        let repo_path = req.repo_path.clone();
        let result = crate::index::index_project_with_embeddings(
            &req.repo_path,
            req.languages,
            req.force_rebuild,
            req.clean_stale,
            EmbeddingOptions {
                enabled: req.embeddings,
                base_url: req.embedding_base_url,
                model: req.embedding_model,
                dimensions: req.embedding_dims,
                api_key: req.embedding_api_key,
            },
        );
        if result.is_ok()
            && let Err(err) = agent::set_current_repo(repo_path)
        {
            return json::<()>(Err(err));
        }
        json(result)
    }

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

    #[tool(
        description = "Return callers, callees, imports, defining file, and resources for one symbol"
    )]
    fn context(&self, Parameters(req): Parameters<ContextRequest>) -> String {
        json(agent::symbol_context(&req.repo_path, &req.symbol))
    }

    #[tool(description = "Estimate upstream and downstream blast radius for a symbol or module")]
    fn impact(&self, Parameters(req): Parameters<ImpactRequest>) -> String {
        json(agent::impact(&req.repo_path, &req.target, req.max_depth))
    }

    #[tool(description = "Map git diff hunks to changed symbols and affected callers/importers")]
    fn detect_changes(&self, Parameters(req): Parameters<ConfigPathRequest>) -> String {
        json(agent::detect_changes(&req.repo_path))
    }

    #[tool(
        description = "Agent-oriented graph search combining lexical matches and module centrality"
    )]
    fn query(&self, Parameters(req): Parameters<QueryRequest>) -> String {
        json(agent::query(&req.repo_path, &req.query, req.limit))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RustRankRouter {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions("RustRank repository analysis tools")
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(
            agent::resources()
                .map(ListResourcesResult::with_all_items)
                .map_err(mcp_internal_error),
        )
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(
            agent::resource_templates(),
        )))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = std::result::Result<ReadResourceResult, McpError>> + Send + '_ {
        let uri = request.uri;
        std::future::ready(
            agent::read_current_resource(&uri)
                .map(|text| {
                    ReadResourceResult::new(vec![
                        ResourceContents::text(text, uri).with_mime_type("text/markdown"),
                    ])
                })
                .map_err(mcp_resource_error),
        )
    }
}

pub fn serve() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--list-tools") {
        println!("{}", ALL_TOOLS.join("\n"));
        return Ok(());
    }

    if std::env::args().nth(1).as_deref() == Some("index-project") {
        return run_index_project_cli();
    }

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match transport_from_env() {
            Transport::StreamableHttp => serve_streamable_http().await,
            Transport::Stdio => serve_stdio().await,
        }
    })
}

fn run_index_project_cli() -> anyhow::Result<()> {
    match parse_index_project_cli(std::env::args().skip(2).collect()) {
        Ok(req) => {
            let repo_path = req.repo_path.clone();
            let response = crate::index::index_project_with_embeddings(
                &req.repo_path,
                req.languages,
                req.force_rebuild,
                req.clean_stale,
                EmbeddingOptions {
                    enabled: req.embeddings,
                    base_url: req.embedding_base_url,
                    model: req.embedding_model,
                    dimensions: req.embedding_dims,
                    api_key: req.embedding_api_key,
                },
            )?;
            agent::set_current_repo(repo_path)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        Err(message) => {
            eprintln!(
                "{}",
                serde_json::json!({
                    "error": true,
                    "code": "INVALID_ARGUMENTS",
                    "message": message,
                    "suggestion": "usage: rustrank index-project --repo-path <path> [--languages python,rust] [--force-rebuild] [--clean-stale] [--embeddings] [--embedding-base-url <url>] [--embedding-model <model>] [--embedding-dims <n>] [--embedding-api-key <key>]"
                })
            );
            std::process::exit(2);
        }
    }
}

fn parse_index_project_cli(args: Vec<String>) -> std::result::Result<IndexProjectRequest, String> {
    let mut repo_path = None;
    let mut languages = None;
    let mut force_rebuild = false;
    let mut clean_stale = false;
    let mut embeddings = None;
    let mut embedding_base_url = None;
    let mut embedding_model = None;
    let mut embedding_dims = None;
    let mut embedding_api_key = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--repo-path" => {
                repo_path = Some(
                    iter.next()
                        .ok_or_else(|| "--repo-path requires a value".to_string())?,
                );
            }
            "--languages" => {
                let raw = iter
                    .next()
                    .ok_or_else(|| "--languages requires a comma-separated value".to_string())?;
                languages = Some(
                    raw.split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                        .collect(),
                );
            }
            "--force-rebuild" => force_rebuild = true,
            "--clean-stale" => clean_stale = true,
            "--embeddings" => embeddings = Some(true),
            "--embedding-base-url" => {
                embedding_base_url = Some(
                    iter.next()
                        .ok_or_else(|| "--embedding-base-url requires a value".to_string())?,
                );
            }
            "--embedding-model" => {
                embedding_model = Some(
                    iter.next()
                        .ok_or_else(|| "--embedding-model requires a value".to_string())?,
                );
            }
            "--embedding-dims" => {
                embedding_dims = Some(
                    iter.next()
                        .ok_or_else(|| "--embedding-dims requires a value".to_string())?
                        .parse()
                        .map_err(|err| format!("invalid --embedding-dims: {err}"))?,
                );
            }
            "--embedding-api-key" => {
                embedding_api_key = Some(
                    iter.next()
                        .ok_or_else(|| "--embedding-api-key requires a value".to_string())?,
                );
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    Ok(IndexProjectRequest {
        repo_path: repo_path.ok_or_else(|| "--repo-path is required".to_string())?,
        languages,
        force_rebuild,
        clean_stale,
        embeddings,
        embedding_base_url,
        embedding_model,
        embedding_dims,
        embedding_api_key,
    })
}

fn default_impact_depth() -> usize {
    2
}

fn default_query_limit() -> usize {
    10
}

fn mcp_internal_error(err: crate::AppError) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

fn mcp_resource_error(err: crate::AppError) -> McpError {
    McpError::resource_not_found(err.to_string(), None)
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
    let http_config = HttpRuntimeConfig::from_env()?;
    let server_config =
        streamable_http_server_config(http_config.allowed_hosts, http_config.allowed_origins);
    let server_config = if http_config.disable_host_check {
        server_config.disable_allowed_hosts()
    } else {
        server_config
    };
    let service: StreamableHttpService<RustRankRouter, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(RustRankRouter::new()),
            Default::default(),
            server_config,
        );
    let app = axum::Router::new()
        .route(HEALTH_PATH, get(|| async { "ok\n" }))
        .nest_service(&http_config.mcp_path, service);
    let listener = tokio::net::TcpListener::bind(http_config.addr).await?;
    eprintln!(
        "RustRank Streamable HTTP listening on http://{}{}",
        listener.local_addr()?,
        http_config.mcp_path
    );
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Debug, Clone)]
struct HttpRuntimeConfig {
    addr: SocketAddr,
    mcp_path: String,
    allowed_hosts: Vec<String>,
    allowed_origins: Vec<String>,
    disable_host_check: bool,
}

impl HttpRuntimeConfig {
    fn from_env() -> anyhow::Result<Self> {
        let listen_addr = env_var("RUSTRANK_LISTEN_ADDR", "RUSTANK_LISTEN_ADDR");
        let host = env_var("RUSTRANK_HOST", "RUSTANK_HOST");
        let port = env_var("RUSTRANK_PORT", "RUSTANK_PORT");
        let addr =
            listen_addr_from_values(listen_addr.as_deref(), host.as_deref(), port.as_deref())?;
        let mcp_path =
            mcp_path_from_value(env_var("RUSTRANK_MCP_PATH", "RUSTANK_MCP_PATH").as_deref())?;

        let mut allowed_hosts = allowed_hosts_for(addr);
        allowed_hosts.extend(parse_csv_list(
            env_var("RUSTRANK_ALLOWED_HOSTS", "RUSTANK_ALLOWED_HOSTS").as_deref(),
        ));
        allowed_hosts.sort();
        allowed_hosts.dedup();

        let allowed_origins = parse_csv_list(
            env_var("RUSTRANK_ALLOWED_ORIGINS", "RUSTANK_ALLOWED_ORIGINS").as_deref(),
        );
        let disable_host_check = bool_from_value(
            env_var("RUSTRANK_DISABLE_HOST_CHECK", "RUSTANK_DISABLE_HOST_CHECK").as_deref(),
        );

        Ok(Self {
            addr,
            mcp_path,
            allowed_hosts,
            allowed_origins,
            disable_host_check,
        })
    }
}

fn streamable_http_server_config(
    allowed_hosts: Vec<String>,
    allowed_origins: Vec<String>,
) -> StreamableHttpServerConfig {
    StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_sse_retry(None)
        .with_allowed_hosts(allowed_hosts)
        .with_allowed_origins(allowed_origins)
}

fn listen_addr_from_values(
    listen_addr: Option<&str>,
    host: Option<&str>,
    port: Option<&str>,
) -> anyhow::Result<SocketAddr> {
    let raw_addr = match non_empty_trimmed(listen_addr) {
        Some(addr) => addr.to_string(),
        None => {
            let host = non_empty_trimmed(host).unwrap_or(DEFAULT_HTTP_HOST);
            let port = non_empty_trimmed(port).unwrap_or(DEFAULT_HTTP_PORT);
            format!("{host}:{port}")
        }
    };

    raw_addr
        .parse()
        .map_err(|err| anyhow::anyhow!("invalid RustRank listen address {raw_addr:?}: {err}"))
}

fn mcp_path_from_value(value: Option<&str>) -> anyhow::Result<String> {
    let raw = non_empty_trimmed(value).unwrap_or(DEFAULT_MCP_PATH);
    let mut path = if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("/{raw}")
    };

    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    if path == "/" {
        return Err(anyhow::anyhow!("RustRank MCP path must not be /"));
    }
    if path == HEALTH_PATH {
        return Err(anyhow::anyhow!(
            "RustRank MCP path {HEALTH_PATH:?} is reserved for health checks"
        ));
    }

    Ok(path)
}

fn parse_csv_list(value: Option<&str>) -> Vec<String> {
    let mut values = Vec::new();
    let Some(value) = value else {
        return values;
    };

    for item in value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        if !values.iter().any(|value| value == item) {
            values.push(item.to_string());
        }
    }

    values
}

fn bool_from_value(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn non_empty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn env_var(primary: &str, legacy: &str) -> Option<String> {
    std::env::var(primary)
        .or_else(|_| std::env::var(legacy))
        .ok()
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

    #[test]
    fn streamable_http_server_config_disables_sse() {
        let config = streamable_http_server_config(vec!["localhost".to_string()], vec![]);

        assert!(!config.stateful_mode);
        assert!(config.json_response);
        assert!(config.sse_keep_alive.is_none());
        assert!(config.sse_retry.is_none());
        assert_eq!(config.allowed_hosts, vec!["localhost"]);
    }

    #[test]
    fn listen_addr_prefers_full_addr_over_host_and_port() {
        let addr = listen_addr_from_values(Some("0.0.0.0:9000"), Some("127.0.0.1"), Some("63477"))
            .expect("addr");

        assert_eq!(addr, "0.0.0.0:9000".parse::<SocketAddr>().expect("parse"));
    }

    #[test]
    fn listen_addr_uses_host_and_port_when_full_addr_missing() {
        let addr = listen_addr_from_values(None, Some("0.0.0.0"), Some("7777")).expect("addr");

        assert_eq!(addr, "0.0.0.0:7777".parse::<SocketAddr>().expect("parse"));
    }

    #[test]
    fn mcp_path_defaults_and_requires_absolute_path() {
        assert_eq!(mcp_path_from_value(None).expect("default"), "/mcp");
        assert_eq!(
            mcp_path_from_value(Some("custom")).expect("normalized"),
            "/custom"
        );
    }

    #[test]
    fn comma_separated_values_are_trimmed_and_deduplicated() {
        let values = parse_csv_list(Some(" api.example.test, localhost, api.example.test ,, "));

        assert_eq!(values, vec!["api.example.test", "localhost"]);
    }
}
