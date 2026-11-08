#!/usr/bin/env python3
"""OpenAI 互換の最小 mock。Agent のツール呼び出しループを実測するためだけのもの。

1 回目: `project_list` の tool_call を返す。
2 回目以降（role=tool を含む）: `narrative` / `hypotheses` の JSON を返す。
実モデルの代用ではなく、**配線が通っていること**だけを確かめる。
"""

from __future__ import annotations

import json
import os
import pathlib
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 11434
CALLS = {"count": 0}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args) -> None:  # 静かにする
        return

    def do_POST(self) -> None:  # noqa: N802 - http.server の API
        length = int(self.headers.get("content-length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        messages = body.get("messages", [])
        trace_path = os.environ.get("MOCK_LLM_TRACE")
        if trace_path:
            path = pathlib.Path(trace_path)
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("a", encoding="utf-8") as handle:
                handle.write(
                    json.dumps(
                        {"tools": body.get("tools", []), "messages": messages},
                        ensure_ascii=False,
                    )
                    + "\n"
                )
        has_tool_result = any(message.get("role") == "tool" for message in messages)
        has_tools = bool(body.get("tools"))
        prompt_text = "\n".join(
            str(message.get("content") or "") for message in messages
        )
        tool_names = {
            tool.get("function", {}).get("name")
            for tool in body.get("tools", [])
        }
        CALLS["count"] += 1

        # 心拍の実測用。この依頼だけ意図的に遅くして job を RUNNING のまま
        # 保持する（WORKER_STALE_SEC を超えさせる）。本番コードには一切影響しない。
        if "MOCK_SLOW_HEARTBEAT" in prompt_text:
            time.sleep(float(os.environ.get("MOCK_LLM_SLOW_SEC", "4")))

        if "MOCK_INVALID_INQUIRY" in prompt_text:
            content = json.dumps(
                {
                    "narrative": "不正な仮説を含む応答。",
                    "hypotheses": [
                        {
                            "claim": "根拠のない仮説",
                            "confidence": 0.5,
                            "basis": ["code:missing"],
                        }
                    ],
                },
                ensure_ascii=False,
            )
            message = {"role": "assistant", "content": content}
            finish = "stop"
        elif has_tool_result or not has_tools:
            content = json.dumps(
                {
                    "narrative": "提示された根拠では、影響はユーザ編集画面周辺に限定されると推定されます。",
                    "hypotheses": [
                        {
                            "claim": "変更はユーザ編集画面に閉じる",
                            "confidence": 0.62,
                            "basis": [
                                "code:d0d0d0d0-d0d0-4d0d-8d0d-d0d0d0d0d0d0"
                            ],
                        }
                    ],
                },
                ensure_ascii=False,
            )
            message = {"role": "assistant", "content": content}
            finish = "stop"
        else:
            selected_tool = "project_list"
            tool_arguments = "{}"
            if "MOCK_KNOWLEDGE_SEARCH" in prompt_text:
                selected_tool = "knowledge_search"
                tool_arguments = json.dumps(
                    {"query": "UserService", "top_k": 3}, ensure_ascii=False
                )
            elif "MOCK_EXTERNAL_DANGEROUS" in prompt_text:
                selected_tool = "dangerous_execute"
            elif "MOCK_EXTERNAL_UNKNOWN" in prompt_text:
                selected_tool = "sql_query"
            elif "MOCK_EXTERNAL_STDIO" in prompt_text:
                selected_tool = "external_stdio_project_list"
            elif "MOCK_EXTERNAL_SCHEMA" in prompt_text:
                selected_tool = "external_schema_project_list"
            elif "MOCK_EXTERNAL_SLOW" in prompt_text:
                selected_tool = "external_slow_project_list"
            elif "MOCK_EXTERNAL_OVERSIZE" in prompt_text:
                selected_tool = "external_oversize_project_list"
            elif "MOCK_EXTERNAL_INVALID" in prompt_text:
                selected_tool = "external_invalid_project_list"
            elif "MOCK_EXTERNAL_HTTP" in prompt_text:
                selected_tool = "external_project_list"
            if selected_tool not in tool_names and selected_tool != "sql_query":
                selected_tool = "project_list"
            message = {
                "role": "assistant",
                "content": None,
                "tool_calls": [
                    {
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": selected_tool,
                            "arguments": tool_arguments,
                        },
                    }
                ],
            }
            finish = "tool_calls"

        payload = json.dumps(
            {
                "id": f"mock-{CALLS['count']}",
                "object": "chat.completion",
                "model": body.get("model", "mock-1"),
                "choices": [{"index": 0, "message": message, "finish_reason": finish}],
                "usage": {"prompt_tokens": 11, "completion_tokens": 7},
            },
            ensure_ascii=False,
        ).encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
