"""Local stdio MCP adapter for the explicitly enabled debug app. Stdlib only."""
import argparse
import base64
import json
import pathlib
import sys
import urllib.error
import urllib.parse
import urllib.request

MAX_LINE = 64 * 1024
MAX_REPLY = 8 * 1024 * 1024


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise urllib.error.HTTPError(newurl, code, "Debug RPC redirects are forbidden", headers, fp)


def schema(properties=None, required=None):
    return {"type": "object", "properties": properties or {}, "required": required or [], "additionalProperties": False}


TEXT = {"type": "string", "maxLength": 128}
BOOL = {"type": "boolean"}
NUMBER = {"type": "number"}
INTEGER = {"type": "integer", "minimum": 0, "maximum": 100000}
SPECS = [
    ("cpu_performance", "performance", "Read a bounded 1..60 second window from the existing profiler cache: CPU/RAM, frame percentiles, slow frames and inclusive stage costs. No GUI repaint or native poll. Explicitly reports unavailable/stopped data and drops.", schema({"seconds":{"type":"integer","minimum":1,"maximum":60}}), True),
    ("cpu_quality", "quality", "Inspect retained original interval delivery and selected CPU/source load quality, plus incomplete CURRENT tracked PID mapping and known errors. History and current roster are separate; unknown reasons stay unknown. Interval pages run newest first.", schema({"offset":INTEGER,"limit":{"type":"integer","minimum":1,"maximum":100}}), True),
    ("cpu_wait", "wait", "Wait up to 6000 ms for a new completed original sample, committed monitoring settings, acknowledged collection release, or the next settled UI frame. new_sample captures the current baseline unless after_seconds is supplied. A normal timeout returns satisfied=false. Settings targets are used only by settings_applied. Pending waits request UI checks every 100 ms and affect timing.", schema({"condition":{"type":"string","enum":["new_sample","settings_applied","collection_released","ui_settled"]},"timeout_ms":{"type":"integer","minimum":1,"maximum":6000},"after_seconds":{"type":"number","minimum":0},"enabled":BOOL,"history_seconds":{"type":"integer","enum":[30,300,1800]}},["condition"]), True),
    ("cpu_state", "state", "Read the rendered graph, controls, groups and monitoring state. No native poll.", schema(), True),
    ("cpu_open_graph", "open_graph", "Open a saved group's graph, or its rule graph using stable IDs from cpu_state.", schema({"group_id": TEXT, "rule_id": TEXT}, ["group_id"]), False),
    ("cpu_close_graph", "close_graph", "Close the CPU modal.", schema(), False),
    ("cpu_select", "select_cpu", "Select a logical CPU in the open graph.", schema({"cpu": INTEGER}, ["cpu"]), False),
    ("cpu_graph_options", "graph_options", "Set graph scope or switch the app Graphs/Processes view.", schema({"all_cpus": BOOL, "tracked_only": BOOL, "view": {"type": "string", "enum": ["graphs", "processes"]}}), False),
    ("cpu_inspect", "inspect", "Pin an exact retained sample by its index back from the newest cut, including pending/gaps.", schema({"samples_back": INTEGER}, ["samples_back"]), False),
    ("cpu_live", "live", "Release inspection and pointer hover, restoring live graph data.", schema(), False),
    ("cpu_monitor", "monitor", "Change ONE monitoring preference in isolated test settings; no affinity enforcement.", schema({"enabled": BOOL, "history_seconds": {"type": "integer", "enum": [30, 300, 1800]}}), False),
    ("cpu_table", "table", "Read a page of the actual rendered contributors or current-app processes table. Open that view first.", schema({"kind": {"type": "string", "enum": ["contributors", "processes"]}, "offset": INTEGER, "limit": {"type": "integer", "minimum": 1, "maximum": 100}}, ["kind"]), True),
    ("cpu_input", "input", "Send egui input to a current rendered control ID. x/y are normalized 0..1; scroll delta is points, positive downward. Re-read cpu_state after layout changes.", schema({"target": TEXT, "kind": {"type": "string", "enum": ["move", "click", "scroll"]}, "x": {"type": "number", "minimum": 0, "maximum": 1}, "y": {"type": "number", "minimum": 0, "maximum": 1}, "delta": {"type": "number", "minimum": -2000, "maximum": 2000}}, ["target", "kind"]), False),
    ("cpu_screenshot", "screenshot", "Capture the actual rendered app frame as PNG. Explicit capture can perturb timing; do not benchmark this frame.", schema(), True),
    ("cpu_close_test_app", "close_app", "Orderly close of this isolated debug app only.", schema(), False),
]


def validate_ticket(ticket):
    if not isinstance(ticket, dict):
        raise ValueError("Invalid local debug session ticket")
    url = urllib.parse.urlsplit(ticket.get("url", ""))
    token = ticket.get("token", "")
    if (url.scheme != "http" or url.hostname != "127.0.0.1" or not url.port or url.path != "/rpc"
            or url.query or url.fragment or url.username or url.password
            or len(token) != 64 or any(c not in "0123456789abcdefABCDEF" for c in token)):
        raise ValueError("Invalid local debug session ticket")
    return ticket


def rpc_from_ticket(path):
    def call(body):
        ticket = validate_ticket(json.loads(pathlib.Path(path).read_text(encoding="utf-8-sig")))
        request = urllib.request.Request(ticket["url"], data=json.dumps(body, allow_nan=False).encode(),
                                         headers={"Content-Type": "application/json", "Authorization": "Bearer " + ticket["token"]})
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        with opener.open(request, timeout=10) as response:
            payload = response.read(MAX_REPLY + 1)
            if len(payload) > MAX_REPLY:
                raise ValueError("App response exceeded budget")
            if response.headers.get_content_type() == "image/png":
                return {"ok": True, "image": base64.b64encode(payload).decode("ascii"), "frame": response.headers.get("X-App-Frame")}
            return json.loads(payload)
    return call


def validate_arguments(spec, args):
    if not isinstance(args, dict) or set(args) - set(spec["properties"]) or set(spec["required"]) - set(args):
        raise ValueError("Arguments do not match the tool schema")
    for key, value in args.items():
        rule = spec["properties"][key]
        kind = rule["type"]
        valid = ((kind == "string" and isinstance(value, str))
                 or (kind == "boolean" and isinstance(value, bool))
                 or (kind == "integer" and isinstance(value, int) and not isinstance(value, bool))
                 or (kind == "number" and isinstance(value, (int, float)) and not isinstance(value, bool)))
        if (not valid or ("enum" in rule and value not in rule["enum"])
                or ("minimum" in rule and value < rule["minimum"])
                or ("maximum" in rule and value > rule["maximum"])
                or ("maxLength" in rule and len(value) > rule["maxLength"])):
            raise ValueError("Invalid argument: " + key)


def dispatch(message, rpc):
    request_id = message.get("id")
    method = message.get("method")
    if request_id is None:
        return None
    if not isinstance(message.get("params", {}), dict):
        return {"jsonrpc": "2.0", "id": request_id, "error": {"code": -32602, "message": "Parameters must be an object"}}
    result = None
    if method == "initialize":
        version = message.get("params", {}).get("protocolVersion")
        supported = {"2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"}
        result = {"protocolVersion": version if version in supported else "2025-06-18", "capabilities": {"tools": {}},
                  "serverInfo": {"name": "cpu-affinity-debug", "version": "0.1.0"},
                  "instructions": "Controls only an explicitly started isolated debug build. Read cpu_state for stable group/rule/control IDs. CPU graph values and tables follow the rendered selected CPU/time; unknown is not zero. Use cpu_input for actual egui widget tests. No OS actions or app launches. Screenshots perturb timing. Session keys are local and must not be printed. The normal release app has no debug endpoint."}
    elif method == "ping":
        result = {}
    elif method == "tools/list":
        result = {"tools": [{"name": name, "description": description, "inputSchema": spec,
                             "annotations": {"readOnlyHint": read_only, "destructiveHint": False, "openWorldHint": False}}
                            for name, _, description, spec, read_only in SPECS]}
    elif method == "tools/call":
        try:
            params = message.get("params", {})
            name = params.get("name")
            found = next((s for s in SPECS if s[0] == name), None)
            if found is None:
                raise ValueError("Unknown tool")
            args = params.get("arguments", {})
            validate_arguments(found[3], args)
            data = rpc({"action": found[1], **args})
            if data.get("image"):
                content = [{"type": "image", "mimeType": "image/png", "data": data["image"]},
                           {"type": "text", "text": json.dumps({"frame": data.get("frame"), "capture_affects_timing": True})}]
            else:
                content = [{"type": "text", "text": json.dumps(data, ensure_ascii=False, allow_nan=False)}]
            result = {"content": content, "isError": not data.get("ok", False)}
        except (ValueError, OSError, urllib.error.URLError, TypeError) as error:
            # Session file contents and authorization headers never enter errors.
            detail = str(error) if isinstance(error, ValueError) else "Debug app unavailable or request failed; start its isolated launcher."
            result = {"content": [{"type": "text", "text": detail}], "isError": True}
    else:
        return {"jsonrpc": "2.0", "id": request_id, "error": {"code": -32601, "message": "Method not found"}}
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--session", required=True)
    args = parser.parse_args()
    rpc = rpc_from_ticket(args.session)
    while True:
        line = sys.stdin.buffer.readline(MAX_LINE + 1)
        if not line:
            break
        if len(line) > MAX_LINE:
            raise SystemExit("MCP input exceeded line budget")
        try:
            message = json.loads(line)
            if not isinstance(message, dict):
                raise ValueError("Request must be an object")
            reply = dispatch(message, rpc)
        except (ValueError, TypeError):
            reply = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "Invalid JSON-RPC input"}}
        if reply is not None:
            sys.stdout.write(json.dumps(reply, ensure_ascii=False, allow_nan=False) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
