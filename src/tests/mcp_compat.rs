use std::{process::Stdio, time::Duration};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

struct Client {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
}

impl Client {
    fn start() -> Self {
        Self::with_env(&[])
    }

    fn with_env(env: &[(&str, &str)]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustrank"))
            .env("RUSTRANK_TRANSPORT", "stdio")
            .env_remove("RUSTRANK_EMBEDDING_BASE_URL")
            .env_remove("RUSTRANK_EMBEDDING_MODEL")
            .env_remove("RUSTRANK_EMBEDDING_DIMS")
            .env_remove("RUSTRANK_EMBEDDING_API_KEY")
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            input,
            output,
        }
    }

    async fn send(&mut self, message: Value) {
        self.input
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        self.input.flush().await.unwrap();
    }

    async fn receive(&mut self) -> Value {
        let line = timeout(Duration::from_secs(10), self.output.next_line())
            .await
            .expect("server response timed out")
            .expect("read response")
            .expect("server exited without a response");
        serde_json::from_str(&line).expect("stdout must contain JSON-RPC only")
    }

    async fn initialize(&mut self) {
        self.send(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":{},
                "clientInfo":{"name":"compat-test","version":"1"}
            }}),
        )
        .await;
        let response = self.receive().await;
        assert_eq!(response["id"], 1);
        assert!(
            response["result"]["capabilities"]["tools"].is_object(),
            "{response}"
        );
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
    }

    async fn stop(mut self) {
        drop(self.input);
        let status = timeout(Duration::from_secs(10), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
    }
}

#[tokio::test]
async fn discovery_probe_allows_legacy_initialize_on_same_process() {
    let mut client = Client::start();
    client
        .send(json!({"jsonrpc":"2.0","id":"server-discover-probe-1",
        "method":"server/discover","params":{}}))
        .await;
    let response = client.receive().await;
    assert_eq!(response["id"], "server-discover-probe-1");
    assert_eq!(response["error"]["code"], -32601);
    client.initialize().await;
    client
        .send(json!({"jsonrpc":"2.0","id":2,"method":"ping"}))
        .await;
    assert_eq!(client.receive().await["result"], json!({}));
    client.stop().await;
}

#[tokio::test]
async fn advertised_tool_properties_are_dictionary_schemas() {
    let mut client = Client::start();
    client.initialize().await;
    client
        .send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await;
    let response = client.receive().await;
    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 19);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        for (name, schema) in tool["inputSchema"]["properties"].as_object().unwrap() {
            assert!(
                schema.is_object(),
                "{}.{} must be a dictionary schema, got {schema}",
                tool["name"],
                name
            );
        }
    }
    client.stop().await;
}

#[tokio::test]
async fn nullable_tool_parameters_use_any_of_instead_of_type_arrays() {
    let mut client = Client::start();
    client.initialize().await;
    client
        .send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await;
    let response = client.receive().await;
    let tools = response["result"]["tools"].as_array().unwrap();
    for (tool_name, field, expected_type) in [
        ("coderank_analysis", "module_prefix", "string"),
        ("contextual_search", "file_type", "string"),
        ("error_patterns", "days_back", "integer"),
        ("index_project", "embeddings", "boolean"),
        ("index_project", "languages", "array"),
    ] {
        let tool = tools.iter().find(|t| t["name"] == tool_name).unwrap();
        let schema = &tool["inputSchema"]["properties"][field];
        assert!(
            schema.get("type").is_none(),
            "{tool_name}.{field}: {schema}"
        );
        let branches = schema["anyOf"]
            .as_array()
            .expect("explicit nullable alternatives");
        assert!(branches.iter().any(|s| s["type"] == expected_type));
        assert!(branches.iter().any(|s| s["type"] == "null"));
        assert!(
            !tool["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!(field))
        );
    }
    client.stop().await;
}

#[tokio::test]
async fn config_value_schema_explicitly_advertises_all_json_types() {
    let mut client = Client::start();
    client.initialize().await;
    client
        .send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await;
    let response = client.receive().await;
    let tool = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "set_config")
        .unwrap();
    let branches = tool["inputSchema"]["properties"]["value"]["anyOf"]
        .as_array()
        .expect("config values must explicitly advertise supported JSON types");
    for kind in ["null", "boolean", "number", "string", "array", "object"] {
        assert!(
            branches.iter().any(|schema| schema["type"] == kind),
            "missing {kind}"
        );
    }
    client.stop().await;
}

#[tokio::test]
async fn initialize_identifies_rustrank_not_its_sdk() {
    let info = rmcp::ServerHandler::get_info(&rustrank::tools::RustRankRouter::new());
    assert_eq!(info.server_info.name, "rustrank");
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.server_info.title.as_deref(), Some("RustRank"));
    assert_eq!(
        info.server_info.website_url.as_deref(),
        Some("https://github.com/midnightphreaker/RustRank")
    );
    assert!(
        info.server_info
            .description
            .as_deref()
            .is_some_and(|s| !s.is_empty())
    );
    let instructions = info.instructions.as_deref().unwrap();
    assert!(instructions.contains("query"));
    assert!(instructions.contains("index_project"));
    assert!(instructions.contains("set_config"));
    let icons = info
        .server_info
        .icons
        .as_ref()
        .expect("server icon metadata");
    assert_eq!(icons.len(), 1);
    assert_eq!(icons[0].mime_type.as_deref(), Some("image/png"));
    assert_eq!(
        icons[0].sizes.as_deref(),
        Some(["256x256".to_owned()].as_slice())
    );
    assert!(icons[0].src.starts_with("data:image/png;base64,"));
    use base64::Engine;
    let encoded = icons[0].src.strip_prefix("data:image/png;base64,").unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(bytes, include_bytes!("../src/assets/rustrank.png"));
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 256);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 256);
}

#[tokio::test]
async fn tool_execution_failures_set_is_error() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join(".rustrank_config.json"), "not json").unwrap();
    let mut client = Client::start();
    client.initialize().await;
    client
        .send(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call", "params":{
                "name":"get_config", "arguments":{"repo_path":repo.path()}
            }}),
        )
        .await;
    let response = client.receive().await;
    assert_eq!(response["result"]["isError"], true, "{response}");
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("error")
    );
    client.stop().await;
}

#[tokio::test]
async fn set_config_preserves_all_json_value_types() {
    let repo = tempfile::tempdir().unwrap();
    let mut client = Client::start();
    client.initialize().await;
    for value in [
        json!(null),
        json!(false),
        json!(42),
        json!(1.5),
        json!("text"),
        json!([1, true]),
        json!({"nested": [null]}),
    ] {
        client.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call", "params":{
            "name":"set_config", "arguments":{"repo_path":repo.path(), "key":"compat", "value":value}
        }})).await;
        let response = client.receive().await;
        assert_ne!(response["result"]["isError"], true, "{response}");
        let config: Value = serde_json::from_str(
            &std::fs::read_to_string(repo.path().join(".rustrank_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["compat"], value);
    }
    client.stop().await;
}

#[tokio::test]
async fn unavailable_embeddings_keep_index_advertised_and_reject_every_argument_shape() {
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("example.rs"), "fn main() {}\n").unwrap();
    let mut client = Client::start();
    client.initialize().await;
    client
        .send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await;
    let response = client.receive().await;
    let tools = response["result"]["tools"].as_array().unwrap();
    let index = tools.iter().find(|t| t["name"] == "index_project").unwrap();
    for key in [
        "embedding_base_url",
        "embedding_model",
        "embedding_dims",
        "embedding_api_key",
    ] {
        assert!(
            index["inputSchema"]["properties"].get(key).is_none(),
            "{key} still exposed"
        );
    }
    for arguments in [
        json!({}),
        json!({"repo_path":42}),
        json!({"repo_path":repo.path(),"force_rebuild":true,"clean_stale":true,"embeddings":false}),
        json!({"embedding_api_key":"ignored-secret","embeddings":true}),
    ] {
        client.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"index_project","arguments":arguments}})).await;
        let response = client.receive().await;
        assert_eq!(response["result"]["isError"], true, "{response}");
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("RUSTRANK_EMBEDDING_BASE_URL"), "{text}");
        assert!(!text.contains("ignored-secret"));
    }
    assert!(!repo.path().join(".rustrank").exists());
    assert!(!repo.path().join("AGENTS.md").exists());
    client
        .send(json!({"jsonrpc":"2.0","id":4,"method":"ping"}))
        .await;
    assert_eq!(client.receive().await["result"], json!({}));
    client.stop().await;
}

async fn index_call(client: &mut Client, arguments: Value) -> Value {
    client.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"index_project","arguments":arguments}})).await;
    client.receive().await
}

#[tokio::test]
async fn invalid_embedding_settings_are_reported_without_stopping_server() {
    for (settings, expected) in [
        (
            vec![("RUSTRANK_EMBEDDING_BASE_URL", "http://127.0.0.1:9/v1")],
            "RUSTRANK_EMBEDDING_MODEL",
        ),
        (
            vec![
                ("RUSTRANK_EMBEDDING_BASE_URL", "http://127.0.0.1:9/v1"),
                ("RUSTRANK_EMBEDDING_MODEL", "test"),
            ],
            "RUSTRANK_EMBEDDING_DIMS",
        ),
    ] {
        let mut client = Client::with_env(&settings);
        client.initialize().await;
        let result = index_call(&mut client, json!({})).await;
        assert_eq!(result["result"]["isError"], true);
        assert!(result.to_string().contains(expected));
        client.stop().await;
    }
    for (url, dims, expected) in [
        ("http://127.0.0.1:9/v1", "0", "positive integer"),
        ("http://127.0.0.1:9/v1", "oops", "positive integer"),
        ("http://127.0.0.1:9/v1", "-1", "positive integer"),
        ("not-a-url", "3", "valid HTTP(S)"),
        ("ftp://example.test/v1", "3", "HTTP(S) API base"),
        (
            "http://user:do-not-leak@example.test/v1",
            "3",
            "without credentials",
        ),
        ("http://127.0.0.1:9/v1", "3", "cannot connect"),
    ] {
        let mut client = Client::with_env(&[
            ("RUSTRANK_EMBEDDING_BASE_URL", url),
            ("RUSTRANK_EMBEDDING_MODEL", "test"),
            ("RUSTRANK_EMBEDDING_DIMS", dims),
        ]);
        client.initialize().await;
        let result = index_call(&mut client, json!({})).await;
        assert_eq!(result["result"]["isError"], true, "{result}");
        assert!(result.to_string().contains(expected), "{result}");
        assert!(!result.to_string().contains("do-not-leak"));
        client.stop().await;
    }
}

#[tokio::test]
async fn embedding_probe_rejects_http_errors_invalid_responses_and_wrong_dimensions() {
    use axum::{Router, http::StatusCode, routing::post};
    for (status, body, expected) in [
        (302, r#"{"data":[{"embedding":[1,2,3]}]}"#, "302"),
        (200, r#"{"data":[{"embedding":[1e100,2,3]}]}"#, "non-finite"),
        (401, r#"{"error":"do-not-leak"}"#, "401"),
        (404, r#"{"error":"unknown model"}"#, "404"),
        (500, "internal error", "500"),
        (200, "not json", "invalid embedding response"),
        (200, r#"{"data":[]}"#, "no data"),
        (200, r#"{"choices":[]}"#, "invalid embedding response"),
        (
            200,
            r#"{"data":[{"embedding":[1,2]}]}"#,
            "dimension mismatch",
        ),
    ] {
        let app = Router::new().route(
            "/v1/embeddings",
            post(move || async move { (StatusCode::from_u16(status).unwrap(), body) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = Client::with_env(&[
            ("RUSTRANK_EMBEDDING_BASE_URL", &url),
            ("RUSTRANK_EMBEDDING_MODEL", "test"),
            ("RUSTRANK_EMBEDDING_DIMS", "3"),
            ("RUSTRANK_EMBEDDING_API_KEY", "do-not-leak"),
        ]);
        client.initialize().await;
        let result = index_call(&mut client, json!({"embeddings":false})).await;
        assert_eq!(result["result"]["isError"], true, "{result}");
        assert!(result.to_string().contains(expected), "{result}");
        assert!(!result.to_string().contains("do-not-leak"));
        client.stop().await;
        task.abort();
    }
}

#[tokio::test]
async fn valid_embedding_endpoint_is_probed_once_and_index_uses_environment() {
    use axum::{Json, Router, http::HeaderMap, routing::post};
    use std::sync::{Arc, Mutex};
    for api_key in [None, Some("test-only-key")] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let app = Router::new().route(
            "/v1/embeddings",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let captured = captured.clone();
                async move {
                    captured.lock().unwrap().push((headers, body));
                    Json(json!({"data":[{"index":0,"embedding":[0.1,0.2,0.3]}]}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut env = vec![
            ("RUSTRANK_EMBEDDING_BASE_URL", url.as_str()),
            ("RUSTRANK_EMBEDDING_MODEL", "fixture-model"),
            ("RUSTRANK_EMBEDDING_DIMS", "3"),
        ];
        if let Some(key) = api_key {
            env.push(("RUSTRANK_EMBEDDING_API_KEY", key));
        }
        let mut client = Client::with_env(&env);
        client.initialize().await;
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "startup must probe before serving"
        );
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("example.rs"), "pub fn example() {}\n").unwrap();
        let args = json!({"repo_path":repo.path(),"force_rebuild":false,"clean_stale":false});
        let result = index_call(&mut client, args.clone()).await;
        assert_ne!(result["result"]["isError"], true, "{result}");
        assert!(repo.path().join(".rustrank/index/v1/embeddings").is_dir());
        let count = requests.lock().unwrap().len();
        assert!(count > 1, "index must generate embeddings by default");
        let mut args = args;
        args["embeddings"] = json!(false);
        let result = index_call(&mut client, args).await;
        assert_ne!(result["result"]["isError"], true, "{result}");
        assert_eq!(
            requests.lock().unwrap().len(),
            count,
            "false must skip vector generation"
        );
        for (headers, body) in requests.lock().unwrap().iter() {
            assert_eq!(body["model"], "fixture-model");
            assert_eq!(body["dimensions"], 3);
            assert_eq!(
                headers.get("authorization").map(|v| v.to_str().unwrap()),
                api_key.map(|_| "Bearer test-only-key")
            );
        }
        client.stop().await;
        task.abort();
    }
}

#[tokio::test]
async fn embedding_probe_timeout_still_allows_initialization() {
    use axum::{Router, routing::post};
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async { std::future::pending::<String>().await }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut client = Client::with_env(&[
        ("RUSTRANK_EMBEDDING_BASE_URL", &url),
        ("RUSTRANK_EMBEDDING_MODEL", "test"),
        ("RUSTRANK_EMBEDDING_DIMS", "3"),
    ]);
    client.initialize().await;
    let result = index_call(&mut client, json!({})).await;
    assert_eq!(result["result"]["isError"], true, "{result}");
    assert!(result.to_string().contains("timed out"), "{result}");
    client.stop().await;
    task.abort();
}
