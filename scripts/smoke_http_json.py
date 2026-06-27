#!/usr/bin/env python3
"""Smoke-test RustRank over no-SSE Streamable HTTP JSON responses."""

from __future__ import annotations

import argparse
import http.server
import json
import subprocess
import sys
import tempfile
import threading
import textwrap
from pathlib import Path
from urllib import error, request

PROTOCOL_VERSION = "2025-06-18"

EXPECTED_TOOLS = [
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
]


class SmokeFailure(RuntimeError):
    pass


class EmbeddingHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        try:
            payload = json.loads(body.decode("utf-8"))
            dims = int(payload.get("dimensions", 3))
        except (ValueError, json.JSONDecodeError):
            dims = 3

        vector = [0.0] * max(dims, 1)
        vector[0] = 1.0
        response = json.dumps({"data": [{"embedding": vector}]}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, format: str, *args: object) -> None:
        return


def start_embedding_server() -> tuple[http.server.ThreadingHTTPServer, str]:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), EmbeddingHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    host, port = server.server_address
    return server, f"http://{host}:{port}/v1"


def write_file(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(content).lstrip(), encoding="utf-8")


def write_fixture(root: Path) -> None:
    root.mkdir(parents=True, exist_ok=True)
    write_file(
        root / "pkg" / "core.py",
        r'''
        import time
        from pkg.models import User

        class Service:
            def login(self, user_id, email):
                if not user_id:
                    raise ValueError("missing user_id")
                for item in range(3):
                    time.sleep(0.01)
                return User(user_id, email)

        def authenticate(user_id, email):
            try:
                svc = Service()
                return svc.login(user_id, email)
            except ValueError as err:
                raise RuntimeError("login failed") from err
        ''',
    )
    write_file(
        root / "pkg" / "models.py",
        r'''
        class User:
            def __init__(self, user_id, email):
                self.user_id = user_id
                self.email = email
        ''',
    )
    write_file(
        root / "pkg" / "api.py",
        r'''
        from pkg.core import authenticate

        def login_endpoint(request):
            user_id = request["user_id"]
            email = request["email"]
            return authenticate(user_id, email)
        ''',
    )
    write_file(
        root / "src" / "lib.rs",
        r'''
        use crate::service::Authenticator;

        pub struct RustUser {
            pub user_id: String,
        }

        pub fn login_user(user_id: &str) -> RustUser {
            if user_id.is_empty() {
                panic!("missing user_id");
            }
            Authenticator::new().login(user_id)
        }
        ''',
    )
    write_file(
        root / "src" / "service.rs",
        r'''
        pub struct Authenticator;

        impl Authenticator {
            pub fn new() -> Self {
                Self
            }

            pub fn login(&self, user_id: &str) -> crate::RustUser {
                crate::RustUser {
                    user_id: user_id.to_string(),
                }
            }
        }
        ''',
    )
    write_file(
        root / "app" / "Controller.cs",
        r'''
        using App.Services;

        namespace App.Controllers;

        public class LoginController {
            private readonly AuthService service = new AuthService();

            public UserDto Login(string userId) {
                if (string.IsNullOrEmpty(userId)) {
                    throw new ArgumentException("missing userId");
                }
                return service.Login(userId);
            }
        }
        ''',
    )
    write_file(
        root / "app" / "AuthService.cs",
        r'''
        namespace App.Services;

        public record UserDto(string UserId);

        public class AuthService {
            public UserDto Login(string userId) {
                return new UserDto(userId);
            }
        }
        ''',
    )
    write_file(
        root / "web" / "auth.ts",
        r'''
        import { AuditLogger } from "./logger";

        export interface Session {
            userId: string;
        }

        export function loginUser(userId: string): Session {
            if (!userId) {
                throw new Error("missing userId");
            }
            AuditLogger.record(userId);
            return { userId };
        }
        ''',
    )
    write_file(
        root / "web" / "logger.tsx",
        r'''
        export class AuditLogger {
            static record(userId: string): void {
                console.log(userId);
            }
        }

        export const LoginView = () => <button>Login</button>;
        ''',
    )
    write_file(
        root / "web" / "auth.js",
        r'''
        const { formatUser } = require("./format");

        export function loginBrowser(userId) {
            if (!userId) {
                throw new Error("missing userId");
            }
            return formatUser(userId);
        }
        ''',
    )
    write_file(
        root / "web" / "format.jsx",
        r'''
        export class Formatter {
            render(userId) {
                return <span>{userId}</span>;
            }
        }

        export const formatUser = (userId) => ({ userId });
        ''',
    )


def maybe_commit_fixture(root: Path) -> None:
    try:
        subprocess.run(["git", "init", "-q"], cwd=root, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(["git", "add", "."], cwd=root, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=RustRank Smoke",
                "-c",
                "user.email=rustrank-smoke@example.invalid",
                "commit",
                "-q",
                "-m",
                "initial fixture",
            ],
            cwd=root,
            check=True,
            stdout=subprocess.DEVNULL,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        pass


def rpc_response(url: str, method: str, params: object | None, request_id: int) -> dict:
    payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        payload["params"] = params
    body = json.dumps(payload).encode("utf-8")
    http_request = request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
            "MCP-Protocol-Version": PROTOCOL_VERSION,
        },
    )

    try:
        with request.urlopen(http_request, timeout=20) as response:
            content_type = response.headers.get("Content-Type", "")
            response_body = response.read().decode("utf-8")
    except error.HTTPError as exc:
        response_body = exc.read().decode("utf-8", errors="replace")
        raise SmokeFailure(f"{method} returned HTTP {exc.code}: {response_body}") from exc
    except error.URLError as exc:
        raise SmokeFailure(f"{method} failed to connect: {exc}") from exc

    if "text/event-stream" in content_type:
        raise SmokeFailure(f"{method} returned SSE content type: {content_type}")
    if "application/json" not in content_type:
        raise SmokeFailure(f"{method} returned non-JSON content type: {content_type}")

    try:
        return json.loads(response_body)
    except json.JSONDecodeError as exc:
        raise SmokeFailure(f"{method} returned invalid JSON: {response_body}") from exc


def rpc(url: str, method: str, params: object | None, request_id: int) -> dict:
    decoded = rpc_response(url, method, params, request_id)

    if decoded.get("error"):
        raise SmokeFailure(f"{method} returned JSON-RPC error: {decoded['error']}")
    if "result" not in decoded:
        raise SmokeFailure(f"{method} returned no result: {decoded}")

    return decoded["result"]


def rpc_error(url: str, method: str, params: object | None, request_id: int) -> dict:
    decoded = rpc_response(url, method, params, request_id)
    error_result = decoded.get("error")
    if not isinstance(error_result, dict):
        raise SmokeFailure(f"{method} returned no JSON-RPC error: {decoded}")
    return error_result


def call_tool(url: str, request_id: int, name: str, arguments: dict) -> None:
    result = rpc(url, "tools/call", {"name": name, "arguments": arguments}, request_id)
    if result.get("isError"):
        raise SmokeFailure(f"{name} returned MCP tool error: {result}")

    content = result.get("content", [])
    if not content:
        raise SmokeFailure(f"{name} returned no content")

    for item in content:
        if item.get("type") != "text":
            continue
        text = item.get("text", "")
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            continue
        if isinstance(parsed, dict) and parsed.get("error"):
                raise SmokeFailure(f"{name} returned RustRank error: {parsed['error']}")


def call_tool_json(url: str, request_id: int, name: str, arguments: dict) -> object:
    result = rpc(url, "tools/call", {"name": name, "arguments": arguments}, request_id)
    if result.get("isError"):
        raise SmokeFailure(f"{name} returned MCP tool error: {result}")

    for item in result.get("content", []):
        if item.get("type") != "text":
            continue
        text = item.get("text", "")
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError:
            continue
        if isinstance(parsed, dict) and parsed.get("error"):
            raise SmokeFailure(f"{name} returned RustRank error: {parsed['error']}")
        return parsed

    raise SmokeFailure(f"{name} returned no JSON text content")


def tool_calls(repo_path: str) -> list[tuple[str, dict]]:
    return [
        (
            "contextual_search",
            {
                "path": repo_path,
                "pattern": "authenticate",
                "file_type": "py",
                "is_regex": False,
                "num_context_lines": 1,
            },
        ),
        (
            "smart_code_search",
            {
                "repo_path": repo_path,
                "pattern": "login",
                "context_lines": 1,
                "num_context_lines": 10,
            },
        ),
        (
            "api_usage",
            {
                "repo_path": repo_path,
                "api_name": "authenticate",
                "max_examples": 10,
                "group_by_pattern": True,
            },
        ),
        (
            "coderank_analysis",
            {
                "repo_path": repo_path,
                "top_n": 20,
                "module_prefix": None,
                "external_modules": True,
            },
        ),
        (
            "code_hotspots",
            {"repo_path": repo_path, "top_n": 10, "min_connections": 1},
        ),
        (
            "trace_data_flow",
            {
                "repo_path": repo_path,
                "identifier": "user_id",
                "include_transformations": True,
                "include_side_effects": True,
            },
        ),
        (
            "trace_feature_impl",
            {"repo_path": repo_path, "feature_keywords": ["login", "authenticate"]},
        ),
        (
            "trace_dep_impact",
            {"repo_path": repo_path, "target_module": "pkg.core"},
        ),
        (
            "error_patterns",
            {
                "repo_path": repo_path,
                "include_antipatterns": True,
                "show_evolution": True,
                "days_back": 36500,
            },
        ),
        (
            "perf_bottleneck",
            {
                "repo_path": repo_path,
                "focus_areas": ["sleep"],
                "include_utility": True,
            },
        ),
        (
            "exec_paths",
            {
                "repo_path": repo_path,
                "function_name": "login",
                "max_depth": 4,
                "include_call_contexts": True,
            },
        ),
        (
            "execute_paths",
            {
                "repo_path": repo_path,
                "function_name": "login",
                "max_depth": 4,
                "include_call_contexts": True,
            },
        ),
        ("get_config", {"repo_path": repo_path}),
        (
            "set_config",
            {"repo_path": repo_path, "key": "smoke_test", "value": {"ok": True}},
        ),
        ("context", {"repo_path": repo_path, "symbol": "authenticate"}),
        (
            "impact",
            {"repo_path": repo_path, "target": "authenticate", "max_depth": 2},
        ),
        ("detect_changes", {"repo_path": repo_path}),
        ("query", {"repo_path": repo_path, "query": "login authenticate", "limit": 5}),
    ]


def exercise_resources(url: str, request_id: int) -> int:
    resources_result = rpc(url, "resources/list", {}, request_id)
    request_id += 1
    resources = resources_result.get("resources", [])
    resource_uris = {resource.get("uri") for resource in resources}
    expected_resources = {
        "rustrank://repo/current/context",
        "rustrank://repo/current/schema",
        "rustrank://repo/current/modules",
        "rustrank://repo/current/processes",
    }
    missing_resources = sorted(expected_resources - resource_uris)
    if missing_resources:
        raise SmokeFailure(f"resource list missing: {missing_resources}")

    templates_result = rpc(url, "resources/templates/list", {}, request_id)
    request_id += 1
    templates = templates_result.get("resourceTemplates", [])
    template_uris = {template.get("uriTemplate") for template in templates}
    expected_templates = {
        "rustrank://repo/current/module/{name}",
        "rustrank://repo/current/process/{name}",
    }
    missing_templates = sorted(expected_templates - template_uris)
    if missing_templates:
        raise SmokeFailure(f"resource template list missing: {missing_templates}")

    resource_reads = [
        ("rustrank://repo/current/context", "RustRank repository context"),
        ("rustrank://repo/current/schema", "RustRank graph schema"),
        ("rustrank://repo/current/modules", "RustRank modules"),
        ("rustrank://repo/current/processes", "RustRank processes"),
    ]

    module_name = None
    process_name = None
    for uri, expected_text in resource_reads:
        read_result = rpc(url, "resources/read", {"uri": uri}, request_id)
        request_id += 1
        contents = read_result.get("contents", [])
        text = "\n".join(item.get("text", "") for item in contents)
        if expected_text not in text:
            raise SmokeFailure(f"resource {uri} did not contain {expected_text!r}: {read_result}")
        if uri.endswith("/modules"):
            module_name = first_backtick_value(text)
            if not contains_fixture_path(text):
                raise SmokeFailure(
                    f"modules resource did not describe the indexed fixture repo: {read_result}"
                )
        if uri.endswith("/processes"):
            process_name = first_backtick_value(text)

    if module_name:
        uri = f"rustrank://repo/current/module/{module_name}"
        read_result = rpc(url, "resources/read", {"uri": uri}, request_id)
        request_id += 1
        text = "\n".join(item.get("text", "") for item in read_result.get("contents", []))
        if f"Module `{module_name}`" not in text:
            raise SmokeFailure(f"module resource {uri} returned unexpected content: {read_result}")
        if not contains_fixture_path(text):
            raise SmokeFailure(f"module resource {uri} did not come from fixture repo: {read_result}")

    if process_name:
        uri = f"rustrank://repo/current/process/{process_name}"
        read_result = rpc(url, "resources/read", {"uri": uri}, request_id)
        request_id += 1
        text = "\n".join(item.get("text", "") for item in read_result.get("contents", []))
        if f"Process `{process_name}`" not in text:
            raise SmokeFailure(f"process resource {uri} returned unexpected content: {read_result}")

    error_result = rpc_error(
        url,
        "resources/read",
        {"uri": "rustrank://repo/current/unknown"},
        request_id,
    )
    request_id += 1
    if "unknown RustRank resource URI" not in json.dumps(error_result):
        raise SmokeFailure(f"unknown resource returned unexpected error: {error_result}")

    return request_id


def first_backtick_value(text: str) -> str | None:
    for line in text.splitlines():
        if "`" not in line:
            continue
        parts = line.split("`")
        if len(parts) >= 3 and parts[1].strip():
            return parts[1].strip()
    return None


def contains_fixture_path(text: str) -> bool:
    return any(
        path in text
        for path in [
            "pkg/core.py",
            "pkg/models.py",
            "src/lib.rs",
            "app/AuthService.cs",
            "web/auth.ts",
        ]
    )


def run_smoke(url: str, repo_path: str, fixture_dir: Path | None = None) -> None:
    request_id = 1
    rpc(
        url,
        "initialize",
        {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "rustrank-smoke", "version": "1.0.0"},
        },
        request_id,
    )
    request_id += 1

    tools_result = rpc(url, "tools/list", {}, request_id)
    request_id += 1
    names = sorted(tool["name"] for tool in tools_result.get("tools", []))
    missing = sorted(set(EXPECTED_TOOLS) - set(names))
    unexpected = sorted(set(names) - set(EXPECTED_TOOLS))
    if missing or unexpected:
        raise SmokeFailure(f"tool list mismatch, missing={missing}, unexpected={unexpected}")

    index_result = call_tool_json(
        url,
        request_id,
        "index_project",
        {
            "repo_path": repo_path,
            "languages": None,
            "force_rebuild": True,
            "clean_stale": True,
        },
    )
    request_id += 1
    if not isinstance(index_result, dict):
        raise SmokeFailure(f"index_project returned unexpected result: {index_result}")
    if index_result.get("indexed_files", 0) <= 0:
        raise SmokeFailure(f"index_project indexed no files: {index_result}")
    if not str(index_result.get("project_manifest", "")).endswith(
        ".rustrank/index/v1/project_manifest.json"
    ):
        raise SmokeFailure(f"index_project returned unexpected manifest: {index_result}")
    if fixture_dir is not None:
        agents_path = fixture_dir / "AGENTS.md"
        if not agents_path.exists():
            raise SmokeFailure("index_project did not create AGENTS.md")
        agents = agents_path.read_text()
        if (
            "<!-- rustrank-index:start -->" not in agents
            or "<!-- rustrank-index:end -->" not in agents
            or "## RustRank Indexed Codebase" not in agents
        ):
            raise SmokeFailure("index_project AGENTS.md section is missing or malformed")

    embedding_server, embedding_base_url = start_embedding_server()
    try:
        embedding_result = call_tool_json(
            url,
            request_id,
            "index_project",
            {
                "repo_path": repo_path,
                "languages": None,
                "force_rebuild": False,
                "clean_stale": True,
                "embeddings": True,
                "embedding_base_url": embedding_base_url,
                "embedding_model": "smoke-embedding-model",
                "embedding_dims": 3,
                "embedding_api_key": "smoke-secret-api-key",
            },
        )
    finally:
        embedding_server.shutdown()
        embedding_server.server_close()
    request_id += 1
    if "smoke-secret-api-key" in json.dumps(embedding_result):
        raise SmokeFailure("index_project embedding API key leaked into tool response")

    request_id = exercise_resources(url, request_id)

    for name, arguments in tool_calls(repo_path):
        call_tool(url, request_id, name, arguments)
        request_id += 1

    print(f"initialized no-SSE Streamable HTTP endpoint: {url}")
    print(f"listed {len(names)} tools")
    print(f"called {len(EXPECTED_TOOLS)} tools successfully")
    print("exercised resources/list, resources/templates/list, and resources/read")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True, help="MCP URL, for example http://127.0.0.1:63477/mcp")
    parser.add_argument(
        "--repo-path",
        help="Repository path as seen by the MCP server. Defaults to the generated fixture path.",
    )
    parser.add_argument(
        "--fixture-dir",
        help="Directory where the test fixture should be created before calling the server.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.fixture_dir:
            fixture_dir = Path(args.fixture_dir).resolve()
            write_fixture(fixture_dir)
            maybe_commit_fixture(fixture_dir)
            run_smoke(args.url, args.repo_path or str(fixture_dir), fixture_dir)
        else:
            with tempfile.TemporaryDirectory(prefix="rustrank-smoke-") as tmp:
                fixture_dir = Path(tmp)
                write_fixture(fixture_dir)
                maybe_commit_fixture(fixture_dir)
                run_smoke(args.url, args.repo_path or str(fixture_dir), fixture_dir)
    except SmokeFailure as exc:
        print(f"smoke test failed: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
