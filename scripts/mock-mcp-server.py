#!/usr/bin/env python3
"""Minimal MCP Server for stdio/http client acceptance.

This is a transport and boundary fixture, not a model of a production server.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PROTOCOL_VERSION = "2024-11-05"


def tool_schema(schema_changed: bool, slow: bool, project_tool_name: str) -> list[dict]:
    input_schema = {
        "type": "object",
        "required": ["project_id"] + (["mode"] if schema_changed else []),
        "properties": {
            "project_id": {"type": "string"},
            **({"mode": {"type": "string"}} if schema_changed else {}),
        },
        "additionalProperties": False,
    }
    tools = [
        {
            "name": project_tool_name,
            "capability": "project.list",
            "description": "External project list fixture",
            "inputSchema": input_schema,
            "outputSchema": {
                "type": "object",
                "required": ["projects"],
                "properties": {
                    "projects": {"type": "array"},
                    "implementation": {"type": "string"},
                },
            },
            "side_effects": "read",
            "timeout_ms": 50 if slow else 2000,
        },
        {
            "name": "dangerous_execute",
            "capability": "test.run_execute",
            "description": "Must never run without approval",
            "inputSchema": {
                "type": "object",
                "required": ["project_id"],
                "properties": {"project_id": {"type": "string"}},
                "additionalProperties": False,
            },
            "outputSchema": {"type": "object"},
            "side_effects": "destructive",
        },
    ]
    return tools


class State:
    def __init__(
        self,
        scenario: str,
        calls_file: pathlib.Path | None,
        project_tool_name: str,
    ) -> None:
        self.scenario = scenario
        self.calls_file = calls_file
        self.project_tool_name = project_tool_name
        self.calls: list[dict] = []

    def record(self, name: str, arguments: dict) -> None:
        entry = {
            "name": name,
            "arguments": arguments,
            "implementation": self.scenario,
        }
        self.calls.append(entry)
        if self.calls_file is not None:
            self.calls_file.parent.mkdir(parents=True, exist_ok=True)
            with self.calls_file.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(entry, ensure_ascii=False) + "\n")

    def tool_result(self, name: str, arguments: dict) -> dict:
        self.record(name, arguments)
        schema_changed = self.scenario == "schema2"
        slow = self.scenario == "slow"
        if slow:
            time.sleep(0.25)
        if self.scenario == "oversize":
            return {
                "content": [{"type": "text", "text": "oversize"}],
                "structuredContent": {
                    "projects": [],
                    "padding": "x" * 300_000,
                },
                "isError": False,
            }
        if self.scenario == "invalid-output":
            return {
                "content": [{"type": "text", "text": "not an object"}],
                "structuredContent": "not an object",
                "isError": False,
            }
        implementation = self.scenario
        output = {
            "projects": [{"project_id": arguments.get("project_id")}],
            "implementation": implementation,
        }
        if schema_changed:
            output["schema"] = "v2"
        return {
            "content": [{"type": "text", "text": json.dumps(output)}],
            "structuredContent": output,
            "isError": False,
        }


def response_for(state: State, request: dict) -> dict | None:
    method = request.get("method")
    request_id = request.get("id")
    if request_id is None:
        return None
    if method == "initialize":
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mock-mcp", "version": "1.0.0"},
            },
        }
    if method == "tools/list":
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "tools": tool_schema(
                    state.scenario == "schema2",
                    state.scenario == "slow",
                    state.project_tool_name,
                )
            },
        }
    if method == "tools/call":
        params = request.get("params", {})
        return {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": state.tool_result(
                str(params.get("name", "")),
                params.get("arguments", {}),
            ),
        }
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": -32601, "message": f"unknown method: {method}"},
    }


def serve_stdio(state: State) -> None:
    for line in sys.stdin:
        if not line.strip():
            continue
        response = response_for(state, json.loads(line))
        if response is not None:
            print(json.dumps(response, ensure_ascii=False), flush=True)


def serve_http(state: State, port: int) -> None:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args) -> None:
            return

        def do_GET(self) -> None:  # noqa: N802
            if self.path != "/calls":
                self.send_error(404)
                return
            payload = json.dumps(state.calls, ensure_ascii=False).encode("utf-8")
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("content-length", "0"))
            request = json.loads(self.rfile.read(length) or b"{}")
            if (
                request.get("method") != "initialize"
                and self.headers.get("mcp-session-id") != "mock-session"
            ):
                self.send_error(400, "missing MCP session")
                return
            response = response_for(state, request)
            if response is None:
                self.send_response(202)
                self.end_headers()
                return
            payload = json.dumps(response, ensure_ascii=False).encode("utf-8")
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(payload)))
            if request.get("method") == "initialize":
                self.send_header("mcp-session-id", "mock-session")
            self.end_headers()
            self.wfile.write(payload)

    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stdio", action="store_true")
    parser.add_argument("--http", type=int)
    parser.add_argument(
        "--scenario",
        choices=["alpha", "beta", "schema2", "slow", "oversize", "invalid-output"],
        default="alpha",
    )
    parser.add_argument("--calls-file", type=pathlib.Path)
    parser.add_argument("--tool-name", default="external_project_list")
    args = parser.parse_args()
    state = State(args.scenario, args.calls_file, args.tool_name)
    if args.http:
        serve_http(state, args.http)
    else:
        serve_stdio(state)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
