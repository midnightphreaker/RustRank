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
        let mut child = Command::new(env!("CARGO_BIN_EXE_rustrank"))
            .env("RUSTRANK_TRANSPORT", "stdio")
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
        ("index_project", "embedding_api_key", "string"),
        ("index_project", "embedding_base_url", "string"),
        ("index_project", "embedding_dims", "integer"),
        ("index_project", "embedding_model", "string"),
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
