"""Small bounded Streamable HTTP client for synthetic built-app acceptance."""
import json
import time
import urllib.parse
import urllib.request

from cpu_affinity_mcp import NoRedirect

MAX_REPLY = 2 * 1024 * 1024


def validate_endpoint(url, token):
    parsed = urllib.parse.urlsplit(url)
    if (parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or not parsed.port
            or parsed.path != "/mcp" or parsed.username or parsed.password
            or parsed.query or parsed.fragment or len(token) != 64
            or any(c not in "0123456789abcdef" for c in token)):
        raise ValueError("Expected an exact synthetic localhost MCP connection")


def decode_reply(content_type, payload, request_id):
    if len(payload) > MAX_REPLY:
        raise ValueError("MCP response exceeded its byte budget")
    if content_type.split(";")[0] == "text/event-stream":
        messages = []
        for block in payload.decode("utf-8").replace("\r\n", "\n").split("\n\n"):
            data = "\n".join(line[5:].lstrip() for line in block.splitlines()
                             if line.startswith("data:"))
            if data:
                messages.append(json.loads(data))
        matches = [m for m in messages if m.get("id") == request_id]
        if len(matches) != 1:
            raise ValueError("MCP stream did not contain one matching response")
        reply = matches[0]
    else:
        reply = json.loads(payload)
    if not isinstance(reply, dict) or reply.get("id") != request_id or "error" in reply:
        raise ValueError("MCP request failed or returned the wrong identity")
    return reply["result"]


def wait_until(label, read, predicate, seconds=15):
    deadline = time.monotonic() + seconds
    while True:
        value = read()
        if predicate(value):
            return value
        if time.monotonic() >= deadline:
            raise AssertionError("Timed out: " + label)
        time.sleep(0.1)


class Client:
    def __init__(self, url, token):
        validate_endpoint(url, token)
        self.url, self.token = url, token
        self.session = None
        self.sequence = 0
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        self.initialize()

    def initialize(self):
        self.session = None
        self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                    "clientInfo": {"name": "owned-live-probe", "version": "1"}})
        self.request("notifications/initialized", notification=True)

    def request(self, method, params=None, notification=False):
        self.sequence += 1
        body = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            body["params"] = params
        if not notification:
            body["id"] = self.sequence
        headers = {"Authorization": "Bearer " + self.token, "Content-Type": "application/json",
                   "Accept": "application/json, text/event-stream", "MCP-Protocol-Version": "2025-06-18"}
        if self.session:
            headers["Mcp-Session-Id"] = self.session
        request = urllib.request.Request(self.url, json.dumps(body).encode(), headers)
        with self.opener.open(request, timeout=10) as response:
            self.session = response.headers.get("Mcp-Session-Id", self.session)
            payload = response.read(MAX_REPLY + 1)
            if notification:
                return None
            return decode_reply(response.headers.get_content_type(), payload, self.sequence)

    def tool(self, name, arguments=None, error=None):
        result = self.request("tools/call", {"name": name, "arguments": arguments or {}})
        data = result.get("structuredContent")
        if data is None:
            data = json.loads(result["content"][0]["text"])
        if error:
            assert result.get("isError") and data["code"] == error, name + " did not refuse the invalid request"
        elif result.get("isError"):
            # Error messages may contain paths/output; keep only the stable failure code in evidence.
            raise AssertionError(name + " failed: " + str(data.get("code", "unknown")))
        return data

    def mutate(self, name, arguments, error=None):
        capabilities = self.tool("get_capabilities")
        args = dict(arguments, request_key=capabilities["app_epoch"] + ":" + capabilities["next_request_sequence"])
        if name in {"create_group", "update_group", "move_group", "create_rule", "update_rule",
                    "move_rule", "update_preferences", "delete_rule", "delete_group", "launch_saved"}:
            args.setdefault("expected_revision", self.tool("get_app_state")["configuration_revision"])
        wire = self.request("tools/call", {"name": name, "arguments": args})
        result = wire.get("structuredContent")
        if result is None:
            result = json.loads(wire["content"][0]["text"])
        if wire.get("isError"):
            assert error and result["code"] == error, name + " admission: " + str(result.get("code"))
            return result
        operation = wait_until(name + " receipt", lambda: self.tool("get_operation", {
            "operation_id": result["operation"]["id"]})["operation"],
            lambda row: row["status"] not in {"accepted", "running"})
        if error:
            assert operation["status"] == "failed" and operation["error"]["code"] == error, name + " did not fail as expected"
            return operation["error"]
        assert operation["status"] == "completed", name + " receipt: " + str((operation.get("error") or {}).get("code"))
        return operation.get("result")
