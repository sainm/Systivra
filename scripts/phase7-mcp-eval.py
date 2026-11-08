#!/usr/bin/env python3
"""Phase 7 external MCP Client acceptance (stdio/http + Agent routing)."""

from __future__ import annotations

import http.cookiejar
import json
import os
import pathlib
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
DB = "systivra_phase7_mcp_verify"
API_PORT = 18089
LLM_PORT = 11535
WEB_PORT = 15175
API_BASE = f"http://127.0.0.1:{API_PORT}"
LLM_BASE = f"http://127.0.0.1:{LLM_PORT}"
WEB_BASE = f"http://localhost:{WEB_PORT}"
PROJECT_ID = "22222222-2222-4222-8222-222222222222"
LOGIN_ID = "alice"
PASSWORD = "SystivraDemo1!"
MOCK_SERVER = ROOT / "scripts" / "mock-mcp-server.py"
LLM_TRACE = ROOT / "data" / "phase7-mcp" / "llm-trace.jsonl"
REPORT: dict = {}


class Client:
    def __init__(self) -> None:
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )

    def call(self, method: str, path: str, body: dict | None = None):
        data = None
        headers = {}
        if body is not None:
            data = json.dumps(body, ensure_ascii=False).encode("utf-8")
            headers["content-type"] = "application/json"
        request = urllib.request.Request(
            f"{API_BASE}/api/v1{path}", data=data, headers=headers, method=method
        )
        try:
            with self.opener.open(request, timeout=30) as response:
                raw = response.read()
                return response.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                parsed = json.loads(raw) if raw else None
            except json.JSONDecodeError:
                parsed = {"raw": raw.decode("utf-8", "replace")}
            return error.code, parsed

    def must(self, method: str, path: str, body: dict | None = None) -> dict:
        status, payload = self.call(method, path, body)
        if status >= 400:
            raise AssertionError(f"{method} {path} -> HTTP {status}: {payload}")
        return payload or {}


def psql(sql: str, database: str = "postgres") -> str:
    result = subprocess.run(
        ["psql", "-d", database, "-v", "ON_ERROR_STOP=1", "-tA", "-c", sql],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"psql failed: {result.stderr.strip()}")
    return result.stdout.strip()


def prepare_db() -> None:
    psql(f"DROP DATABASE IF EXISTS {DB};")
    psql(f"CREATE DATABASE {DB};")
    for migration in sorted((ROOT / "migrations").glob("0*.sql")):
        result = subprocess.run(
            ["psql", "-d", DB, "-v", "ON_ERROR_STOP=1", "-q", "-f", str(migration)],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(f"{migration.name}: {result.stderr.strip()}")
    subprocess.run(
        ["psql", "-d", DB, "-q", "-f", str(ROOT / "deployment" / "postgres" / "roles.sql")],
        check=True,
        capture_output=True,
    )
    env = dict(os.environ, PGDATABASE=DB)
    env["PATH"] = f"{pathlib.Path.home()}/.cargo/bin:{env.get('PATH', '')}"
    subprocess.run(
        [str(ROOT / "scripts" / "seed-dev.sh")],
        check=True,
        capture_output=True,
        env=env,
    )


def wait_http(url: str, attempts: int = 120) -> None:
    for _ in range(attempts):
        try:
            urllib.request.urlopen(url, timeout=1).close()
            return
        except Exception:  # noqa: BLE001
            time.sleep(0.1)
    raise RuntimeError(f"{url} did not become ready")


def wait_job(client: Client, job_id: str) -> dict:
    terminal = {"SUCCEEDED", "SUCCEEDED_WITH_ERRORS", "FAILED", "CANCELLED", "TIMEOUT"}
    for _ in range(240):
        job = client.must("GET", f"/jobs/{job_id}")
        if job["status"] in terminal:
            if job["status"] in {"FAILED", "CANCELLED", "TIMEOUT"}:
                raise AssertionError(f"job {job_id} ended as {job['status']}: {job.get('errors')}")
            return job
        time.sleep(0.25)
    raise TimeoutError(job_id)


def start_http_mock(
    scenario: str,
    port: int,
    tool_name: str,
    calls_file: pathlib.Path,
) -> subprocess.Popen:
    process = subprocess.Popen(
        [
            sys.executable,
            str(MOCK_SERVER),
            "--http",
            str(port),
            "--scenario",
            scenario,
            "--tool-name",
            tool_name,
            "--calls-file",
            str(calls_file),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_http(f"http://127.0.0.1:{port}/calls")
    return process


def stop_process(process: subprocess.Popen | None) -> None:
    if process is None or process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()


def json_lines(path: pathlib.Path) -> list[dict]:
    if not path.exists():
        return []
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def server_view(client: Client, name: str) -> dict:
    servers = client.must("GET", f"/projects/{PROJECT_ID}/mcp/servers")["items"]
    return next(item for item in servers if item["name"] == name)


def discover(client: Client, name: str) -> dict:
    return client.must(
        "POST", f"/projects/{PROJECT_ID}/mcp/servers/{name}/discover"
    )


def register(client: Client, name: str, transport: str, config: dict) -> dict:
    return client.must(
        "POST",
        f"/projects/{PROJECT_ID}/mcp/servers",
        {"name": name, "transport": transport, "config": config},
    )


def run_agent(client: Client, question: str, agent_code: str = "external-agent") -> dict:
    accepted = client.must(
        "POST",
        f"/projects/{PROJECT_ID}/agents/{agent_code}/runs",
        {"question": question},
    )
    wait_job(client, accepted["job_id"])
    return client.must("GET", f"/projects/{PROJECT_ID}/agent-runs/{accepted['run_id']}")


def main() -> int:
    api = worker = llm = vite = None
    http_alpha = http_beta = schema_alpha = schema_beta = None
    slow_http = oversize_http = invalid_http = None
    try:
        prepare_db()
        for path in (
            ROOT / "data" / "phase7-mcp" / "http-calls.jsonl",
            ROOT / "data" / "phase7-mcp" / "stdio-calls.jsonl",
            LLM_TRACE,
        ):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.unlink(missing_ok=True)

        http_calls = ROOT / "data" / "phase7-mcp" / "http-calls.jsonl"
        schema_calls = ROOT / "data" / "phase7-mcp" / "schema-calls.jsonl"
        stdio_calls = ROOT / "data" / "phase7-mcp" / "stdio-calls.jsonl"
        slow_calls = ROOT / "data" / "phase7-mcp" / "slow-calls.jsonl"
        oversize_calls = ROOT / "data" / "phase7-mcp" / "oversize-calls.jsonl"
        invalid_calls = ROOT / "data" / "phase7-mcp" / "invalid-calls.jsonl"

        http_alpha = start_http_mock("alpha", 18100, "external_project_list", http_calls)
        schema_alpha = start_http_mock(
            "alpha", 18101, "external_schema_project_list", schema_calls
        )
        slow_http = start_http_mock(
            "slow", 18102, "external_slow_project_list", slow_calls
        )
        oversize_http = start_http_mock(
            "oversize", 18103, "external_oversize_project_list", oversize_calls
        )
        invalid_http = start_http_mock(
            "invalid-output", 18104, "external_invalid_project_list", invalid_calls
        )

        llm_env = dict(
            os.environ,
            MOCK_LLM_TRACE=str(LLM_TRACE),
        )
        llm = subprocess.Popen(
            [sys.executable, str(ROOT / "scripts" / "mock-llm.py"), str(LLM_PORT)],
            env=llm_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(0.4)
        ai_env = {
            "AI_BASE_URL": f"{LLM_BASE}/v1",
            "AI_MODEL_REASONING": "mock-1",
            "AI_MODEL_FAST": "mock-1",
            "MOCK_LLM_TRACE": str(LLM_TRACE),
        }
        api_env = dict(os.environ, **ai_env)
        api_env.update(
            {
                "PORT": str(API_PORT),
                "DATABASE_URL": f"postgres://systivra_app@localhost:5432/{DB}",
                "SESSION_SECRET": "phase7-mcp-secret-0123456789abcdef0123456789abcdef",
                "SESSION_COOKIE_SECURE": "false",
                "PUBLIC_BASE_URL": API_BASE,
                "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
                "NEO4J_DISABLED": "1",
            }
        )
        api = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "systivra-api")],
            env=api_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        wait_http(f"{API_BASE}/healthz")
        worker_env = dict(os.environ, **ai_env)
        worker_env.update(
            {
                "DATABASE_URL": f"postgres://systivra_scheduler@localhost:5432/{DB}",
                "WORKER_TICK_SEC": "1",
                "SYSTIVRA_DATA_DIR": str(ROOT / "data"),
                "NEO4J_DISABLED": "1",
            }
        )
        worker = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "systivra-worker")],
            env=worker_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        client = Client()
        login = client.must(
            "POST", "/auth/login", {"login_id": LOGIN_ID, "password": PASSWORD}
        )
        assert login["memberships"]
        status, invalid = client.call(
            "POST",
            f"/projects/{PROJECT_ID}/mcp/servers",
            {"name": "bad", "transport": "http", "config": {}},
        )
        assert status == 400, (status, invalid)

        client.must(
            "POST",
            f"/projects/{PROJECT_ID}/agents",
            {
                "code": "external-agent",
                "model_profile": "reasoning",
                "prompt_ref": "prompt://external-agent@1",
                "prompt_body": "外部 MCP Tool の結果を確認し、narrative と hypotheses を返す。",
                "capabilities": ["project.list", "test.run_execute"],
                "budgets": {
                    "max_tool_calls": 8,
                    "max_tokens": 100000,
                    "max_wall_time_sec": 120,
                    "max_cost_usd": 1.0,
                },
            },
        )
        client.must(
            "POST",
            f"/projects/{PROJECT_ID}/agents",
            {
                "code": "limited-agent",
                "model_profile": "fast",
                "prompt_ref": "prompt://limited-agent@1",
                "prompt_body": "Use only the allowed capability.",
                "capabilities": ["coverage.get"],
                "budgets": {
                    "max_tool_calls": 2,
                    "max_tokens": 10000,
                    "max_wall_time_sec": 60,
                    "max_cost_usd": 1.0,
                },
            },
        )

        http_config = {
            "url": "http://127.0.0.1:18100/mcp",
            "timeout_ms": 2000,
            "headers": {"X-Mock-Client": "systivra"},
        }
        register(client, "http-tools", "http", http_config)
        http_view = discover(client, "http-tools")
        assert http_view["health_status"] == "UP", http_view
        assert http_view["tool_count"] == 2, http_view
        assert http_view["tools"][0]["server"] == "http-tools", http_view["tools"]
        http_hash = http_view["schema_hash"]

        alpha_run = run_agent(client, "MOCK_EXTERNAL_HTTP alpha")
        assert alpha_run["status"] == "SUCCEEDED", alpha_run
        assert any(
            step["name"] == "external_project_list" and step["status"] == "PASSED"
            for step in alpha_run["steps"]
        ), alpha_run["steps"]
        alpha_calls = json_lines(http_calls)
        assert alpha_calls[-1]["implementation"] == "alpha", alpha_calls
        assert alpha_calls[-1]["arguments"]["project_id"] == PROJECT_ID
        overreach = run_agent(client, "MOCK_EXTERNAL_HTTP overreach", "limited-agent")
        assert overreach["status"] == "FAILED", overreach
        assert overreach["error"]["code"] == "AGENT_CAPABILITY_DENIED", overreach

        stop_process(http_alpha)
        http_alpha = None
        http_beta = start_http_mock("beta", 18100, "external_project_list", http_calls)
        register(client, "http-tools", "http", http_config)
        beta_view = discover(client, "http-tools")
        assert beta_view["health_status"] == "UP", beta_view
        assert beta_view["schema_hash"] == http_hash, beta_view
        beta_run = run_agent(client, "MOCK_EXTERNAL_HTTP beta")
        assert beta_run["status"] == "SUCCEEDED", beta_run
        beta_calls = json_lines(http_calls)
        assert beta_calls[-1]["implementation"] == "beta", beta_calls
        agents = client.must("GET", f"/projects/{PROJECT_ID}/agents")["items"]
        external_agent = next(item for item in agents if item["code"] == "external-agent")
        assert external_agent["current_version"] == 1, external_agent
        REPORT["http_replacement"] = {
            "schema_stable": beta_view["schema_hash"] == http_hash,
            "implementations": [alpha_calls[-1]["implementation"], beta_calls[-1]["implementation"]],
            "agent_version": external_agent["current_version"],
            "project_id_injected": beta_calls[-1]["arguments"]["project_id"] == PROJECT_ID,
        }

        schema_config = {"url": "http://127.0.0.1:18101/mcp", "timeout_ms": 2000}
        register(client, "schema-tools", "http", schema_config)
        schema_v1 = discover(client, "schema-tools")
        assert schema_v1["health_status"] == "UP", schema_v1
        old_hash = schema_v1["schema_hash"]
        old_project_tool = next(
            tool for tool in schema_v1["tools"] if tool["name"] == "external_schema_project_list"
        )
        assert "mode" not in old_project_tool["input_schema"].get("required", [])
        stop_process(schema_alpha)
        schema_alpha = None
        schema_beta = start_http_mock(
            "schema2", 18101, "external_schema_project_list", schema_calls
        )
        schema_v2 = discover(client, "schema-tools")
        assert schema_v2["health_status"] == "DEGRADED", schema_v2
        assert schema_v2["schema_hash"] != old_hash, schema_v2
        assert schema_v2["health_detail"] == "MCP_SCHEMA_CHANGED", schema_v2
        retained_tool = next(
            tool for tool in schema_v2["tools"] if tool["name"] == "external_schema_project_list"
        )
        assert "mode" not in retained_tool["input_schema"].get("required", [])
        assert discover(client, "schema-tools")["health_status"] == "DEGRADED"
        changed_audits = int(
            psql(
                "SELECT count(*) FROM core.audit_log "
                "WHERE action='ai.mcp.discover' AND result='DEGRADED' "
                "AND detail->>'schema_changed'='true';",
                DB,
            )
            or 0
        )
        assert changed_audits >= 1, changed_audits

        register(client, "schema-recovered-tools", "http", schema_config)
        schema_recovered = discover(client, "schema-recovered-tools")
        assert schema_recovered["health_status"] == "UP", schema_recovered
        accepted_tool = next(
            tool
            for tool in schema_recovered["tools"]
            if tool["name"] == "external_schema_project_list"
        )
        assert "mode" in accepted_tool["input_schema"].get("required", [])
        REPORT["schema_change"] = {
            "old_hash": old_hash,
            "new_hash": schema_v2["schema_hash"],
            "degraded": schema_v2["health_status"],
            "old_tools_retained": True,
            "explicit_reregister_recovered": schema_recovered["health_status"],
            "audit_rows": changed_audits,
        }

        stdio_config = {
            "command": sys.executable,
            "args": [
                str(MOCK_SERVER),
                "--stdio",
                "--scenario",
                "alpha",
                "--tool-name",
                "external_stdio_project_list",
                "--calls-file",
                str(stdio_calls),
            ],
            "timeout_ms": 2000,
        }
        register(client, "stdio-tools", "stdio", stdio_config)
        stdio_view = discover(client, "stdio-tools")
        assert stdio_view["health_status"] == "UP", stdio_view
        assert stdio_view["transport"] == "stdio"
        stdio_run = run_agent(client, "MOCK_EXTERNAL_STDIO")
        assert stdio_run["status"] == "SUCCEEDED", stdio_run
        stdio_called = json_lines(stdio_calls)
        assert stdio_called[-1]["name"] == "external_stdio_project_list", stdio_called
        assert stdio_called[-1]["arguments"]["project_id"] == PROJECT_ID
        REPORT["stdio"] = {
            "health": stdio_view["health_status"],
            "tools": stdio_view["tool_count"],
            "call": stdio_called[-1],
        }

        slow_config = {
            "url": "http://127.0.0.1:18102/mcp",
            "timeout_ms": 2000,
        }
        register(client, "slow-tools", "http", slow_config)
        assert discover(client, "slow-tools")["health_status"] == "UP"
        slow_run = run_agent(client, "MOCK_EXTERNAL_SLOW")
        slow_step = next(
            step for step in slow_run["steps"] if step["name"] == "external_slow_project_list"
        )
        assert slow_step["status"] == "FAILED", slow_step
        slow_server = server_view(client, "slow-tools")
        assert slow_server["health_status"] == "DOWN", slow_server
        assert "timed out" in slow_server["health_detail"], slow_server

        oversize_config = {
            "url": "http://127.0.0.1:18103/mcp",
            "timeout_ms": 2000,
        }
        register(client, "oversize-tools", "http", oversize_config)
        assert discover(client, "oversize-tools")["health_status"] == "UP"
        oversize_run = run_agent(client, "MOCK_EXTERNAL_OVERSIZE")
        oversize_step = next(
            step
            for step in oversize_run["steps"]
            if step["name"] == "external_oversize_project_list"
        )
        assert oversize_step["status"] == "FAILED", oversize_step
        oversize_server = server_view(client, "oversize-tools")
        assert oversize_server["health_status"] == "DOWN", oversize_server
        assert "exceeds" in oversize_server["health_detail"], oversize_server

        invalid_config = {
            "url": "http://127.0.0.1:18104/mcp",
            "timeout_ms": 2000,
        }
        register(client, "invalid-tools", "http", invalid_config)
        assert discover(client, "invalid-tools")["health_status"] == "UP"
        invalid_run = run_agent(client, "MOCK_EXTERNAL_INVALID")
        invalid_step = next(
            step
            for step in invalid_run["steps"]
            if step["name"] == "external_invalid_project_list"
        )
        assert invalid_step["status"] == "FAILED", invalid_step
        invalid_server = server_view(client, "invalid-tools")
        assert invalid_server["health_status"] == "DEGRADED", invalid_server
        assert "schema violation" in invalid_server["health_detail"], invalid_server

        register(
            client,
            "down-tools",
            "http",
            {"url": "http://127.0.0.1:18110/mcp", "timeout_ms": 300},
        )
        down = discover(client, "down-tools")
        assert down["health_status"] == "DOWN", down
        assert down["health_detail"], down

        unknown = run_agent(client, "MOCK_EXTERNAL_UNKNOWN")
        assert unknown["status"] == "FAILED", unknown
        assert unknown["error"]["code"] == "CAPABILITY_UNSUPPORTED", unknown

        before_dangerous = len(json_lines(http_calls))
        dangerous = run_agent(client, "MOCK_EXTERNAL_DANGEROUS")
        assert dangerous["status"] == "FAILED", dangerous
        assert dangerous["error"]["code"] == "POLICY_APPROVAL_REQUIRED", dangerous
        after_dangerous = json_lines(http_calls)
        assert len(after_dangerous) == before_dangerous, after_dangerous
        assert all(call["name"] != "dangerous_execute" for call in after_dangerous)

        traces = json_lines(LLM_TRACE)
        tool_messages = [
            message
            for trace in traces
            for message in trace.get("messages", [])
            if message.get("role") == "tool"
        ]
        assert any(
            '"untrusted":true' in message.get("content", "")
            and '"mcp_server":"http-tools"' in message.get("content", "")
            for message in tool_messages
        ), tool_messages[-3:]
        REPORT["failures"] = {
            "timeout": slow_server["health_status"],
            "oversize": oversize_server["health_status"],
            "invalid_output": invalid_server["health_status"],
            "down": down["health_status"],
            "unknown_tool": unknown["error"]["code"],
            "overreach": overreach["error"]["code"],
            "approval": dangerous["error"]["code"],
            "approval_did_not_call_server": True,
            "untrusted_isolated": True,
        }

        vite_env = dict(os.environ, VITE_API_PROXY=API_BASE)
        vite = subprocess.Popen(
            [
                str(ROOT / "apps" / "web" / "node_modules" / ".bin" / "vite"),
                "--port",
                str(WEB_PORT),
                "--strictPort",
            ],
            cwd=ROOT / "apps" / "web",
            env=vite_env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        wait_http(f"{WEB_BASE}/")
        ui_result = subprocess.run(
            ["npm", "run", "check:ui:phase7-mcp"],
            cwd=ROOT / "apps" / "web",
            env=dict(os.environ, WEB_BASE=WEB_BASE, PROJECT_ID=PROJECT_ID),
            capture_output=True,
            text=True,
        )
        if ui_result.returncode != 0:
            raise RuntimeError(f"phase7 MCP UI failed:\n{ui_result.stdout}\n{ui_result.stderr}")
        ui_line = next(
            (
                line
                for line in reversed(ui_result.stdout.splitlines())
                if line.startswith("{")
            ),
            "{}",
        )
        REPORT["ui"] = json.loads(ui_line)

        audits = int(
            psql(
                "SELECT count(*) FROM core.audit_log WHERE action LIKE 'ai.mcp.%';",
                DB,
            )
            or 0
        )
        assert audits >= 10, audits
        REPORT["audit_rows"] = audits

        out = ROOT / "data" / "phase7-mcp" / "report.json"
        out.write_text(json.dumps(REPORT, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(REPORT, ensure_ascii=False, indent=2))
        print(f"\nwritten: {out}")
        return 0
    finally:
        for process in (
            vite,
            worker,
            api,
            llm,
            http_alpha,
            http_beta,
            schema_alpha,
            schema_beta,
            slow_http,
            oversize_http,
            invalid_http,
        ):
            stop_process(process)
        try:
            psql(f"DROP DATABASE IF EXISTS {DB};")
        except Exception:  # noqa: BLE001
            pass


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001
        print(f"FAILED: {error}", file=sys.stderr)
        raise
