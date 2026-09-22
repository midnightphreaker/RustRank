pub mod agent;
pub mod analysis;
pub mod code_rank;
mod compat;
pub mod config;
pub mod index;
pub mod search;
pub mod trace;

use std::{future::Future, net::SocketAddr};

use crate::embeddings::{EmbeddingConfig, EmbeddingOptions};
use axum::routing::get;
use base64::Engine;
use clap::{Parser, Subcommand, error::ErrorKind};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Icon, Implementation, ListResourceTemplatesResult,
        ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult,
        ResourceContents, ServerCapabilities, ServerInfo,
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
const INDEX_PROJECT_USAGE: &str = "usage: rustrank index-project --repo-path <path> [--languages python,rust] [--force-rebuild] [--clean-stale] [--embeddings] [--embedding-base-url <url>] [--embedding-model <model>] [--embedding-dims <n>] [--embedding-api-key <key>]";

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
    embedding_config: Result<EmbeddingConfig, String>,
}

impl RustRankRouter {
    pub fn new() -> Self {
        let embedding_config = crate::embeddings::validated_server_config().map_err(|problem| {
            format!("Embeddings unavailable: {problem}. Configure RUSTRANK_EMBEDDING_BASE_URL, RUSTRANK_EMBEDDING_MODEL and RUSTRANK_EMBEDDING_DIMS (RUSTRANK_EMBEDDING_API_KEY is optional), then restart RustRank.")
        });
        if let Err(problem) = &embedding_config {
            eprintln!("RustRank DEBUG: {problem}");
        }
        Self {
            tool_router: Self::tool_router(),
            embedding_config,
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
    #[schemars(default, schema_with = "nullable_schema::<String>")]
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
    #[schemars(default, schema_with = "nullable_schema::<String>")]
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
    #[schemars(default, schema_with = "nullable_schema::<u32>")]
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
    #[schemars(schema_with = "config_value_schema")]
    value: serde_json::Value,
}

fn config_value_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    // Enumerate all JSON types instead of a boolean or unconstrained schema.
    // `number` includes integers; arrays and objects remain free-form.
    schemars::json_schema!({
        "description": "Configuration value (any JSON value)",
        "anyOf": [
            {"type": "null"},
            {"type": "boolean"},
            {"type": "number"},
            {"type": "string"},
            {"type": "array"},
            {"type": "object", "additionalProperties": true}
        ]
    })
}

fn nullable_schema<T: schemars::JsonSchema>(
    generator: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    // Preserve T's constraints and nullability without the less portable type array.
    schemars::json_schema!({"anyOf": [generator.subschema_for::<T>(), {"type": "null"}]})
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IndexProjectRequest {
    repo_path: String,
    #[schemars(default, schema_with = "nullable_schema::<Vec<String>>")]
    languages: Option<Vec<String>>,
    force_rebuild: bool,
    clean_stale: bool,
    #[serde(default)]
    #[schemars(schema_with = "nullable_schema::<bool>")]
    embeddings: Option<bool>,
}

#[derive(Debug, Parser)]
#[command(
    name = "rustrank",
    about = "RustRank MCP server and repository indexing CLI"
)]
struct Cli {
    /// Print the registered MCP tool names and exit.
    #[arg(long)]
    list_tools: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Index a repository into RustRank's persistent cache.
    IndexProject(IndexProjectCli),
}

#[derive(Debug, Parser)]
struct IndexProjectCli {
    /// Repository path to index.
    #[arg(long, value_name = "PATH")]
    repo_path: String,

    /// Comma-separated language names to index.
    #[arg(long, value_delimiter = ',', value_name = "LANGUAGES")]
    languages: Vec<String>,

    /// Rebuild all cache entries even when file hashes are unchanged.
    #[arg(long)]
    force_rebuild: bool,

    /// Remove stale cache entries for files no longer present.
    #[arg(long)]
    clean_stale: bool,

    /// Enable embedding generation for indexed symbols.
    #[arg(long)]
    embeddings: bool,

    /// Embedding API base URL.
    #[arg(long, value_name = "URL")]
    embedding_base_url: Option<String>,

    /// Embedding model name.
    #[arg(long, value_name = "MODEL")]
    embedding_model: Option<String>,

    /// Embedding vector dimensionality.
    #[arg(long, value_name = "N")]
    embedding_dims: Option<usize>,

    /// Embedding API key.
    #[arg(long, value_name = "KEY")]
    embedding_api_key: Option<String>,
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
        description = "Build or refresh repository indexes and generated AGENTS.md guidance. Embeds bounded function/source chunks using the server’s RUSTRANK_EMBEDDING_* configuration. If embeddings are unavailable, still builds the structural index and returns a warning. embeddings defaults on; false skips vectors. Returns counts, cache statistics and warnings; clean_stale removes obsolete structural cache entries."
    )]
    fn index_project(&self, Parameters(req): Parameters<IndexProjectRequest>) -> CallToolResult {
        let options = match &self.embedding_config {
            Ok(config) => EmbeddingOptions {
                enabled: Some(req.embeddings.unwrap_or(true)),
                base_url: Some(config.base_url.clone()),
                model: Some(config.model.clone()),
                dimensions: Some(config.dimensions),
                api_key: config.api_key.clone(),
            },
            Err(_) => EmbeddingOptions {
                enabled: Some(false),
                ..Default::default()
            },
        };
        let repo_path = req.repo_path.clone();
        let mut result = crate::index::index_project_with_embeddings(
            &req.repo_path,
            req.languages,
            req.force_rebuild,
            req.clean_stale,
            options,
        );
        if let Ok(response) = &mut result
            && let Err(problem) = &self.embedding_config
        {
            response.warnings.push(format!(
                "Embeddings skipped; structural index built. {problem}"
            ));
        }
        if result.is_ok()
            && let Err(err) = agent::set_current_repo(repo_path)
        {
            return json::<()>(Err(err));
        }
        json(result)
    }

    #[tool(
        description = "Find literal text or regex matches under a directory. Returns file paths, 1-based line numbers, matching lines and surrounding context. file_type accepts an extension such as rs or .py; omit it to search enabled source languages. Respects configured exclusions."
    )]
    fn contextual_search(
        &self,
        Parameters(req): Parameters<ContextualSearchRequest>,
    ) -> CallToolResult {
        json(search::contextual_search(
            &req.path,
            &req.pattern,
            req.file_type.as_deref(),
            req.is_regex,
            req.num_context_lines,
        ))
    }

    #[tool(
        description = "Find literal text in supported source files, ranked by import-graph PageRank. Returns matching lines, file paths, surrounding context and scores. Use contextual_search for regex or extension filtering. context_lines sets context per match; num_context_lines caps the number of results, with a minimum of one."
    )]
    fn smart_code_search(
        &self,
        Parameters(req): Parameters<SmartCodeSearchRequest>,
    ) -> CallToolResult {
        json(search::smart_code_search(
            &req.repo_path,
            &req.pattern,
            req.context_lines,
            req.num_context_lines,
        ))
    }

    #[tool(
        description = "Find literal occurrences of an API name to learn local usage conventions. Returns up to max_examples file/line snippets, optionally labeled call, import, assignment or reference. Classification is text-based, not resolved API identity, so check matches before copying a pattern."
    )]
    fn api_usage(&self, Parameters(req): Parameters<ApiUsageRequest>) -> CallToolResult {
        json(search::api_usage(
            &req.repo_path,
            &req.api_name,
            req.max_examples,
            req.group_by_pattern,
        ))
    }

    #[tool(
        description = "Identify structurally important modules across supported languages using import-graph PageRank. Returns module names, scores, outgoing import counts and incoming importer counts (depth). Filter by module_prefix; enable external_modules to include unresolved imports. Scores measure graph importance, not code quality or runtime cost."
    )]
    fn coderank_analysis(&self, Parameters(req): Parameters<CodeRankRequest>) -> CallToolResult {
        json(code_rank::coderank_analysis(
            &req.repo_path,
            req.top_n,
            req.module_prefix.as_deref(),
            req.external_modules,
        ))
    }

    #[tool(
        description = "Prioritize modules for review using import-graph importance weighted by a change-frequency estimate. Returns module scores, import counts and change_frequency; min_connections filters weakly connected modules. Frequency uses distinct Git blame commits where available, otherwise textual references, so it is a heuristic rather than a churn metric."
    )]
    fn code_hotspots(&self, Parameters(req): Parameters<HotspotRequest>) -> CallToolResult {
        json(code_rank::code_hotspots(
            &req.repo_path,
            req.top_n,
            req.min_connections,
        ))
    }

    #[tool(
        description = "Locate whole-word identifier occurrences across parsed source files. Returns file/line snippets labeled definition, usage, transformation or side_effect, plus inferred layers. The include flags enable extra classifications; they do not filter ordinary usages. This is textual tracing, not alias-aware data-flow or taint analysis."
    )]
    fn trace_data_flow(&self, Parameters(req): Parameters<DataFlowRequest>) -> CallToolResult {
        json(trace::trace_data_flow(
            &req.repo_path,
            &req.identifier,
            req.include_transformations,
            req.include_side_effects,
        ))
    }

    #[tool(
        description = "Locate a feature by case-insensitive keyword matches in supported source files. Returns file/line snippets, the first matching keyword and a layer inferred from the file path: api, data, tests, ui or business_logic. Use this to find candidate implementation sites, then inspect their symbols with context."
    )]
    fn trace_feature_impl(&self, Parameters(req): Parameters<FeatureRequest>) -> CallToolResult {
        let keywords = req
            .feature_keywords
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        json(trace::trace_feature_impl(&req.repo_path, &keywords))
    }

    #[tool(
        description = "Find modules that directly import target_module, using language-aware local import resolution. Returns import-site file/line snippets and target-to-dependent chains. Supply a module name from query or coderank_analysis. Use impact to explore callers beyond direct imports; this tool does not traverse transitive dependencies."
    )]
    fn trace_dep_impact(&self, Parameters(req): Parameters<DepImpactRequest>) -> CallToolResult {
        json(trace::trace_dep_impact(&req.repo_path, &req.target_module))
    }

    #[tool(
        description = "Scan source lines for try/except, raise and throw; optionally flag unwrap and panic patterns. Returns file/line snippets with pattern and heuristic severity. show_evolution adds Git blame commit counts within days_back when available, not historical error diffs. This is a pattern scan, not exhaustive error-handling analysis."
    )]
    fn error_patterns(&self, Parameters(req): Parameters<ErrorPatternsRequest>) -> CallToolResult {
        json(analysis::error_patterns(
            &req.repo_path,
            req.include_antipatterns,
            req.show_evolution,
            req.days_back,
        ))
    }

    #[tool(
        description = "Find source lines containing performance-related keywords and return file/line snippets with heuristic severity. focus_areas supplies case-insensitive terms; an empty list uses sleep, range, append and push. include_utility=false skips paths containing util. Findings are review candidates, not measured bottlenecks or profiling results."
    )]
    fn perf_bottleneck(&self, Parameters(req): Parameters<PerfRequest>) -> CallToolResult {
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

    #[tool(
        description = "Inspect a function by exact name for branch, loop, error-path and optional call-like source lines. Returns file/line snippets labeled by kind. max_depth limits reported findings per matching function, not call-stack depth. This is a static line scan, not executable path enumeration."
    )]
    fn exec_paths(&self, Parameters(req): Parameters<ExecPathsRequest>) -> CallToolResult {
        json(analysis::exec_paths(
            &req.repo_path,
            &req.function_name,
            req.max_depth,
            req.include_call_contexts,
        ))
    }

    #[tool(
        description = "Compatibility alias for exec_paths with identical arguments and results. Prefer exec_paths for new calls. Scans exact-name function matches for branch, loop, error-path and optional call-like lines; max_depth caps findings per function, not call-stack depth."
    )]
    fn execute_paths(&self, Parameters(req): Parameters<ExecPathsRequest>) -> CallToolResult {
        json(analysis::execute_paths(
            &req.repo_path,
            &req.function_name,
            req.max_depth,
            req.include_call_contexts,
        ))
    }

    #[tool(
        description = "Read the repository's .rustrank_config.json as a JSON object; returns an empty object when the file is absent. Use before changing language, exclusion or embedding settings. This returns stored configuration, not merged defaults or environment overrides, and does not modify files."
    )]
    fn get_config(&self, Parameters(req): Parameters<ConfigPathRequest>) -> CallToolResult {
        json(config::get_config(&req.repo_path))
    }

    #[tool(
        description = "Persist a value in .rustrank_config.json and return the updated configuration. key is a dotted path such as languages.enabled; value accepts any JSON type. Creates the file if needed and replaces the selected value. Read get_config first to inspect existing settings; this does not rebuild indexes."
    )]
    fn set_config(&self, Parameters(req): Parameters<SetConfigRequest>) -> CallToolResult {
        json(config::set_config(&req.repo_path, &req.key, req.value))
    }

    #[tool(
        description = "Inspect a symbol before editing it. Returns its defining file and line span, module, kind, callers, callees, imports and related MCP resource URIs. Call relationships are static heuristics with confidence labels; verify them in source. Use query first if you do not know the symbol name."
    )]
    fn context(&self, Parameters(req): Parameters<ContextRequest>) -> CallToolResult {
        json(agent::symbol_context(&req.repo_path, &req.symbol))
    }

    #[tool(
        description = "Estimate change impact for a symbol or module. Returns affected callers/importers as graph nodes and edges with distance and confidence, plus any stale-index warning. max_depth bounds caller traversal; import relationships are direct. Use before changing shared code; this is a static estimate, not proof of all dependencies."
    )]
    fn impact(&self, Parameters(req): Parameters<ImpactRequest>) -> CallToolResult {
        json(agent::impact(&req.repo_path, &req.target, req.max_depth))
    }

    #[tool(
        description = "Review unstaged tracked-file changes against the Git index. Returns changed files, symbols overlapping added/modified lines, affected callers/importers and a heuristic risk level. Requires a Git working tree. Staged-only changes and untracked files are excluded; deletions may lack symbol mappings, so also review git diff."
    )]
    fn detect_changes(&self, Parameters(req): Parameters<ConfigPathRequest>) -> CallToolResult {
        json(agent::detect_changes(&req.repo_path))
    }

    #[tool(
        description = "Find relevant modules and symbols from whitespace-separated search terms. Ranks matches using names, paths, source text and importer counts, with chunk-level semantic matches using the server’s embedding configuration. Falls back to text and graph matching when embeddings are unavailable. Returns file/line locations, match reasons, scores, resource URIs and process hints. Start here for exploration; use contextual_search for exact or regex matches."
    )]
    fn query(&self, Parameters(req): Parameters<QueryRequest>) -> CallToolResult {
        json(agent::query_with_embeddings(
            &req.repo_path,
            &req.query,
            req.limit,
            self.embedding_config.as_ref().ok(),
        ))
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
        .with_server_info(
            Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
                .with_title("RustRank")
                .with_description(
                    "Repository analysis for LLMs: source indexing, code search, import-graph ranking, symbol context and change-impact inspection across supported languages.",
                )
                .with_website_url("https://github.com/midnightphreaker/RustRank")
                .with_icons(vec![
                    Icon::new(format!(
                        "data:image/png;base64,{}",
                        base64::engine::general_purpose::STANDARD
                            .encode(include_bytes!("../assets/rustrank.png")),
                    ))
                    .with_mime_type("image/png")
                    .with_sizes(vec!["256x256".to_owned()]),
                ]),
        )
        .with_instructions(
            "Use repo_path for the repository directory on the server. Start with query to locate relevant code, context to inspect a symbol, and impact before changing shared code. Use contextual_search for literal or regex matches; detect_changes reviews unstaged tracked-file edits. index_project writes repository-local caches and generated guidance and selects the repo for MCP resources; set_config writes repository configuration. Embedding-enabled operations may call the configured API. Analysis is static and partly heuristic: verify findings in source. Tool results contain JSON text; execution failures set isError.",
        )
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
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() > 1 {
        let is_index_project = args.get(1).is_some_and(|arg| arg == "index-project");
        let cli = match Cli::try_parse_from(args) {
            Ok(cli) => cli,
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) =>
            {
                err.print()?;
                return Ok(());
            }
            Err(err) if is_index_project => {
                print_index_project_invalid_arguments(err.to_string());
                std::process::exit(2);
            }
            Err(err) => {
                err.print()?;
                std::process::exit(2);
            }
        };

        if cli.list_tools {
            println!("{}", ALL_TOOLS.join("\n"));
            return Ok(());
        }

        if let Some(Commands::IndexProject(req)) = cli.command {
            return run_index_project_cli(req);
        }
    }

    let router = RustRankRouter::new();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match transport_from_env() {
            Transport::StreamableHttp => serve_streamable_http(router).await,
            Transport::Stdio => serve_stdio(router).await,
        }
    })
}

fn run_index_project_cli(req: IndexProjectCli) -> anyhow::Result<()> {
    let repo_path = req.repo_path.clone();
    let response = crate::index::index_project_with_embeddings(
        &req.repo_path,
        (!req.languages.is_empty()).then_some(req.languages),
        req.force_rebuild,
        req.clean_stale,
        EmbeddingOptions {
            enabled: req.embeddings.then_some(true),
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

fn print_index_project_invalid_arguments(message: String) {
    eprintln!(
        "{}",
        serde_json::json!({
            "error": true,
            "code": "INVALID_ARGUMENTS",
            "message": message,
            "suggestion": INDEX_PROJECT_USAGE
        })
    );
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

async fn serve_stdio(router: RustRankRouter) -> anyhow::Result<()> {
    let (read, write) = rmcp::transport::stdio();
    let transport = rmcp::transport::async_rw::AsyncRwTransport::new_server(read, write);
    let service = router
        .serve(compat::LegacyDiscoveryTransport(transport))
        .await?;
    service.waiting().await?;
    Ok(())
}

async fn serve_streamable_http(router: RustRankRouter) -> anyhow::Result<()> {
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
            move || Ok(router.clone()),
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

fn json<T: serde::Serialize>(result: crate::Result<T>) -> CallToolResult {
    let serialized = result
        .map_err(|err| err.to_string())
        .and_then(|value| serde_json::to_string(&value).map_err(|err| err.to_string()));
    match serialized {
        Ok(text) => CallToolResult::success(vec![Content::text(text)]),
        Err(error) => CallToolResult::error(vec![Content::text(
            serde_json::json!({ "error": error }).to_string(),
        )]),
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
