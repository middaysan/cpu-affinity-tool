"""Named acceptance cases using the real app's HTTP API and rendered debug controls."""
import csv
import base64
import ctypes
import io
import json
import math
import socket
import subprocess
import time

from live_mcp_client import Client, wait_until
from live_mcp_native import OwnedProcess, available_cpus, listener_pid, pin_workload


def controls(state):
    return {row["id"]: row["extra"] for row in state["controls"]}


def steam_selection(fixture, client, workload):
    """Missing Steam metadata must open the existing chooser, never launch Steam."""
    cpu = available_cpus()[-1]
    group = client.mutate('create_group', {'name': 'Steam detection', 'cpus': [cpu]})['group_id']
    # This deliberately uninstalled AppID has no metadata; no URI is dispatched.
    rule = {'name': 'Sample Steam game', 'steam_app_id': 4294967294, 'steam_executable': None,
            'args': [], 'tracked_process_names': [workload.name], 'manage_descendants': False,
            'autorun': False, 'priority': 'Normal'}
    rule_id = client.mutate('create_rule', {'group_id': group, 'rule': rule})['rule_id']
    warning = 'steam.warning.' + rule_id
    select = 'steam.select.' + rule_id
    wait_until('actionable Steam warning', fixture.state,
               lambda s: warning in controls(s) and select in controls(s))
    for theme, index in [('dark', 2), ('light', 1)]:
        if index == 1:
            fixture.click('settings.open')
            fixture.click('settings.theme.1')
            fixture.click('settings.open')
        fixture.call('input', target='nav.groups', kind='move', x=0.05, y=0.1)
        assert fixture.call('wait', condition='ui_settled', timeout_ms=6000)['satisfied']
        frame = fixture.rpc({'action': 'screenshot'})
        assert frame.get('ok') and frame.get('image')
        (fixture.root / ('steam-warning-' + theme + '.png')).write_bytes(base64.b64decode(frame['image']))
    original = fixture.state_path.read_bytes()
    fixture.click(warning)
    wait_until('warning opens chooser with reason', fixture.state,
               lambda s: 'steam.picker' in controls(s) and 'steam.detection_reason' in controls(s))
    frame = fixture.rpc({'action': 'screenshot'})
    assert frame.get('ok') and frame.get('image')
    (fixture.root / 'steam-selection.png').write_bytes(base64.b64decode(frame['image']))
    fixture.click('rule.form.close')
    assert fixture.state_path.read_bytes() == original, 'Opening/closing warning changed the saved rule'
    stop = fixture.root / 'steam-workload.stop'
    process = subprocess.Popen([str(workload), str(stop)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    fixture.children.append((process, stop))
    fixture.click(select)
    fixture.click('steam.process.' + str(process.pid))
    assert fixture.state_path.read_bytes() == original, 'Choosing a draft process saved prematurely'
    fixture.call('input', target='steam.choose_process', kind='scroll', delta=700)
    assert fixture.call('wait', condition='ui_settled', timeout_ms=6000)['satisfied']
    fixture.click('rule.form.save')
    wait_until('saved exact Steam executable', lambda: client.tool('list_rules', {'group_id': group}),
               lambda value: value['rows'][0]['app']['launch_target']['Steam']['executable'] == str(workload))
    saved = client.tool('list_rules', {'group_id': group})['rows'][0]['app']
    assert saved['additional_processes'] == [workload.name], 'Selection rewrote helper names'
    wait_until('Steam warning clears after exact choice', fixture.state,
               lambda s: warning not in controls(s) and 'rule.form.close' not in controls(s))
    client.mutate('update_preferences', {'preferences': {'cpu_load_monitoring_enabled': True}})
    wait_until('exact owned game is measured', fixture.state,
               lambda s: controls(s).get('meter.rule.' + rule_id, {}).get('instances') == 1
               and controls(s).get('meter.rule.' + rule_id, {}).get('measured') == 1,
               seconds=20)
    return {'warning_actionable': True, 'reason_visible': True, 'chooser_opened': True,
            'draft_not_published_early': True, 'exact_choice_measured': True, 'helper_names_preserved': True}


def assert_unions(rows, group, first, second):
    cpu = [rows["meter." + key] for key in ["group." + group, "rule." + first, "rule." + second]]
    assert cpu[0]["sample_unix_ms"] and len({r["sample_unix_ms"] for r in cpu}) == 1, "Mixed current CPU samples"
    values = [r["cpu_percent"] for r in cpu]
    assert all(v is not None and math.isfinite(v) for v in values), "Unavailable CPU is not zero"
    assert max(values) - min(values) < 0.001, "Shared process CPU was double counted"
    memory = [rows["memory." + key]["private_bytes"] for key in ["group." + group, "rule." + first, "rule." + second]]
    assert all(v is not None for v in memory) and len(set(memory)) == 1, "Shared process RAM was double counted"


def connect(fixture):
    fixture.click("mcp.status")
    try:
        files = wait_until("isolated authority", lambda: list((fixture.root / "a").glob("mcp-authority/*/access.json")), bool)
    except AssertionError:
        image = fixture.rpc({"action": "screenshot"})
        if image.get("image"):
            (fixture.root / "bootstrap-failure.png").write_bytes(base64.b64decode(image["image"]))
        raise
    assert len(files) == 1
    policy_path = files[0]
    policy = json.loads(policy_path.read_bytes())
    assert policy["enabled"] is False, "MCP must start Off"
    fixture.close()
    # Only this fresh diagnostic profile is provisioned. Shipping uses the real Connection UI.
    with socket.socket() as available:
        available.bind(("127.0.0.1", 0))
        policy["port"] = available.getsockname()[1]
    policy_path.write_text(json.dumps(policy), encoding="utf-8")
    fixture.start()
    fixture.click("mcp.status")
    fixture.click("settings.production_mcp")
    def client_ready():
        try:
            owner = listener_pid(policy["port"])
            if owner is None:
                return None
            assert fixture.process.poll() is None and owner == fixture.process.pid, "Foreign listener; key not sent"
            client = Client("http://127.0.0.1:" + str(policy["port"]) + "/mcp", policy["token"])
            assert client.tool("get_capabilities")["process_id"] == fixture.process.pid, "Wrong listener process"
            return client
        except (OSError, ValueError):
            return None
    client = wait_until("production MCP listening", client_ready, bool)
    fixture.click("mcp.connection.close")
    return client


def configuration(fixture, client, main, helper):
    topology = client.tool("get_topology")
    cpu = max(c for c in available_cpus() if c < topology["logical_cpu_count"])
    first = client.mutate("create_group", {"name": "Owned A", "cpus": [cpu]})["group_id"]
    second = client.mutate("create_group", {"name": "Owned B", "cpus": [cpu]})["group_id"]
    rule = {"name": "Owned root and extra", "path": str(main), "args": [],
            "tracked_process_names": [main.name, helper.name], "manage_descendants": False,
            "autorun": False, "priority": "Normal"}
    rule_id = client.mutate("create_rule", {"group_id": first, "rule": rule})["rule_id"]
    # A real incomplete replacement must be refused without changing its saved peer fields.
    before = fixture.state_path.read_bytes()
    client.mutate("update_rule", {"group_id": first, "rule_id": rule_id, "rule": {"name": "bad"}}, error="validation")
    assert fixture.state_path.read_bytes() == before, "Invalid replacement changed disk state"
    stale = client.tool("get_app_state")["configuration_revision"]
    rule["name"] = "Renamed owned rule"
    client.mutate("update_rule", {"group_id": first, "rule_id": rule_id, "rule": rule})
    client.mutate("update_group", {"group_id": first, "name": "stale", "cpus": [cpu], "expected_revision": stale}, error="revision_conflict")
    client.mutate("move_group", {"group_id": second, "index": 0})
    client.mutate("move_rule", {"group_id": first, "rule_id": rule_id, "target_group_id": second, "index": 0})
    saved = client.tool("list_rules", {"group_id": second})["rows"][0]
    assert saved["rule_id"] == rule_id
    assert saved["app"]["additional_processes"] == rule["tracked_process_names"]
    assert saved["app"]["priority"] == "Normal" and saved["app"]["args"] == []
    assert client.tool("list_groups")["rows"][0]["group_id"] == second
    durable = json.loads(fixture.state_path.read_bytes())
    fixture.close()
    fixture.start()
    # The persisted permanent key/port and enabled state must survive restart.
    def restarted():
        try:
            port = int(client.url.split(":")[2].split("/")[0])
            owner = listener_pid(port)
            if owner is None:
                return None
            assert owner == fixture.process.pid and fixture.process.poll() is None
            client.initialize()
            assert client.tool("get_capabilities")["process_id"] == fixture.process.pid
            return client.tool("get_app_state")
        except (OSError, ValueError):
            return None
    wait_until("MCP restart", restarted, lambda state: state and state["group_count"] == 2)
    assert json.loads(fixture.state_path.read_bytes()) == durable
    assert client.tool("list_rules", {"group_id": second})["rows"][0]["rule_id"] == rule_id
    return first, second, rule_id, cpu


def upgrade(fixture):
    original = fixture.state_path.read_bytes()
    fixture.click("nav.tweaks")
    wait_until("upgrade notice", fixture.state, lambda s: any(c["id"] == "tweaks.settings_update" for c in s["controls"]))
    assert fixture.state_path.read_bytes() == original, "Opening old settings wrote the file"
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetFileAttributesW.argtypes = [ctypes.c_wchar_p]
    kernel.GetFileAttributesW.restype = ctypes.c_uint32
    kernel.SetFileAttributesW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32]
    kernel.SetFileAttributesW.restype = ctypes.c_int
    attrs = kernel.GetFileAttributesW(str(fixture.state_path))
    assert attrs != 0xffffffff and kernel.SetFileAttributesW(str(fixture.state_path), attrs | 1)
    try:
        fixture.click("tweaks.settings_update")
        wait_until("read-only save refusal", fixture.state, lambda s: "Storage" in s["tweaks"]["operation"])
        assert fixture.state_path.read_bytes() == original, "Failed update changed the old file"
    finally:
        assert kernel.SetFileAttributesW(str(fixture.state_path), attrs)
    fixture.click("tweaks.settings_update")
    wait_until("settings update completion", fixture.state,
               lambda s: s["tweaks"]["operation"] == "Idle" and not s["tweaks"]["operation_error"]
               and not any(c["id"] == "tweaks.settings_update" for c in s["controls"]))
    saved, old = json.loads(fixture.state_path.read_bytes()), json.loads(original)
    assert saved["version"] == 14 and saved["groups"] == old["groups"]
    assert not fixture.state()["tweaks"]["has_changes"] and fixture.state()["tweaks"]["previous_owned"] == []
    fixture.close()
    fixture.start()
    assert json.loads(fixture.state_path.read_bytes()) == saved


def telemetry(fixture, client, main, helper, group, peer_group, rule_id, cpu):
    rule = {"name": "Shared identical set", "path": str(helper), "args": [],
            "tracked_process_names": [main.name, helper.name], "manage_descendants": False,
            "autorun": False, "priority": "Normal"}
    shared = client.mutate("create_rule", {"group_id": group, "rule": rule})["rule_id"]
    peer = client.mutate("create_rule", {"group_id": peer_group, "rule": rule})["rule_id"]
    client.mutate("update_preferences", {"preferences": {"cpu_load_monitoring_enabled": True}})
    workloads = []
    for path in [main, helper]:
        stop = fixture.root / (path.stem + ".stop")
        child = subprocess.Popen([str(path), str(stop)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        fixture.children.append((child, stop))
        workloads.append((child, stop))
        pin_workload(child, cpu)
    def measured(state):
        rows = controls(state)
        return all(rows.get("meter." + key, {}).get("instances") == 2
                   and rows.get("meter." + key, {}).get("measured") == 2
                   and (rows.get("meter." + key, {}).get("cpu_percent") or 0) > 0
                   and (rows.get("memory." + key, {}).get("private_bytes") or 0) > 0
                   and rows.get("memory." + key, {}).get("instances") == 2
                   and rows.get("memory." + key, {}).get("measured") == 2
                   for key in ["group." + group, "rule." + rule_id, "rule." + shared,
                               "group." + peer_group, "rule." + peer])
    wait_until("root/helper measured", fixture.state, measured, seconds=25)
    seen = set()
    def samples():
        state = fixture.state()
        if measured(state):
            rows = controls(state)
            assert_unions(rows, group, rule_id, shared)
            assert_unions(rows, peer_group, rule_id, peer)
            seen.add(rows["meter.group." + group]["sample_unix_ms"])
        return len(seen)
    wait_until("three independent CPU/RAM union samples", samples, lambda n: n >= 3, seconds=15)
    fixture.call("open_graph", group_id=group)
    graph = fixture.state()["graph"]
    fixture.call("close_graph")
    # Stop root: an explicitly added helper must keep both rule and group measured.
    process, stop = workloads[0]
    stop.write_text("stop", encoding="utf-8")
    process.wait(timeout=5)
    wait_until("extra process survives primary exit", fixture.state,
               lambda s: controls(s).get("meter.rule." + rule_id, {}).get("instances") == 1
               and controls(s).get("meter.rule." + rule_id, {}).get("measured") == 1
               and (controls(s).get("meter.rule." + rule_id, {}).get("cpu_percent") or 0) > 0
               and controls(s).get("memory.rule." + rule_id, {}).get("instances") == 1
               and controls(s).get("memory.rule." + rule_id, {}).get("measured") == 1)
    fixture.call("monitor", enabled=False)
    assert fixture.call("wait", condition="collection_released", timeout_ms=6000)["satisfied"]
    assert not any(key.startswith(("meter.group.", "meter.rule.")) for key in controls(fixture.state()))
    fixture.call("monitor", enabled=True)
    wait_until("current values recover after On", fixture.state,
               lambda s: controls(s).get("meter.rule." + rule_id, {}).get("measured") == 1)
    return {"fresh_union_samples": len(seen), "graph_opened": bool(graph), "cpu": cpu}


def history(fixture, client, cpu, helper):
    query = {"source": "system", "cpu": cpu, "limit": 2}
    history = wait_until("system history", lambda: client.tool("read_cpu_history", query),
                         lambda row: any(r["delivery"] == "completed" for r in row.get("rows", [])), seconds=20)
    original = next(row for row in history["rows"] if row["delivery"] == "completed")
    contributor = client.tool("read_cpu_contributors", {"source": "system", "cpu": cpu, "seconds": original["seconds"]})
    assert any(row["name"].lower() == helper.name.lower() and row["load"] > 0
               and row["historical_lifetime_id"] for row in contributor["rows"]), "Owned helper missing from original-time contributors"
    prepared = client.tool("export_cpu_history")
    ready = wait_until("CSV preparation", lambda: client.tool("export_cpu_history", {
        "export_id": prepared["export_id"], "limit": 128}), lambda r: r["status"] == "ready")
    chunks, total = [], 0
    deadline = time.monotonic() + 30
    while True:
        assert time.monotonic() < deadline, "CSV download exceeded its total deadline"
        chunks.append(ready["text"])
        total += len(ready["text"].encode("utf-8"))
        assert total <= 2 * 1024 * 1024, "Tiny fixture export exceeded its budget"
        if ready["eof"]:
            break
        assert int(ready["next_after"]) > int(ready["after"]), "CSV cursor made no progress"
        ready = client.tool("export_cpu_history", {"export_id": ready["export_id"],
                            "after": ready["next_after"], "limit": 128})
    text = "".join(chunks)
    rows = list(csv.DictReader(io.StringIO(text.lstrip("\ufeff"))))
    assert rows and all(row["time_utc"] and row["elapsed_seconds"] for row in rows)
    assert any(row["record_type"] == "cpu" for row in rows)
    assert any(row["record_type"] == "process" and row["process"].lower() == helper.name.lower()
               and float(row["cpu_percent"]) > 0 and row["historical_lifetime_id"] for row in rows)
    fixture.call("monitor", enabled=False)
    assert fixture.call("wait", condition="collection_released", timeout_ms=6000)["satisfied"]
    retained = client.tool("export_cpu_history", {"export_id": ready["export_id"], "limit": 128})
    assert retained["status"] == "ready", "Off destroyed an immutable export"
    client.tool("read_cpu_history", dict(query, epoch="foreign:0"), error="cursor_reset")
    return {"csv_rows": len(rows), "csv_bytes": total, "history_rows": len(history["rows"])}


def execution(fixture, client, executable, cpu):
    stop = fixture.root / "managed.stop"
    launched = client.mutate("launch_program", {"path": str(executable), "args": [str(stop)],
                             "cpus": [cpu], "priority": "BelowNormal", "capture": True})
    child = OwnedProcess(launched["pid"], launched["creation_token"])
    fixture.children.append((child, stop))
    assert launched["elevated"] is False, "Default managed launch inherited elevation"
    assert child.scheduling() == (1 << cpu, 0x4000), "External scheduling readback differs"
    exact = {"pid": launched["pid"], "creation_token": launched["creation_token"]}
    inspected = client.tool("inspect_process", exact)
    assert int(inspected["affinity_mask"]) == 1 << cpu
    output = wait_until("managed output", lambda: client.tool("read_run_output", {
        "run_id": launched["run_id"], "stream": "stdout"}), lambda row: "owned workload ready" in row["text"])
    assert not output["cursor_gap"]
    # A deliberately wrong creation token must never terminate the real child.
    bad = dict(exact, creation_token=str(int(exact["creation_token"]) + 1))
    capabilities = client.tool("get_capabilities")
    bad["request_key"] = capabilities["app_epoch"] + ":" + capabilities["next_request_sequence"]
    wire = client.request("tools/call", {"name": "stop_process", "arguments": bad})
    if not wire.get("isError"):
        receipt = wire["structuredContent"]["operation"]["id"]
        result = wait_until("wrong-token refusal", lambda: client.tool("get_operation", {
            "operation_id": receipt})["operation"], lambda op: op["status"] not in {"accepted", "running"})
        assert result["status"] == "failed"
    assert not client.tool("inspect_process", exact)["exited"]
    focus = client.mutate("focus_process", exact)
    # Windows controls foreground permission. Record actual acknowledgement, not an invented focus guarantee.
    stop.write_text("stop", encoding="utf-8")
    assert child.wait(timeout=5) == 0
    drained = wait_until("output EOF", lambda: client.tool("read_run_output", {
        "run_id": launched["run_id"], "stream": "stdout", "after": output["next"]}), lambda row: row["eof"])
    assert drained["read_error"] is None
    return {"ordinary_launch": True, "native_affinity_priority": True,
            "wrong_lifetime_refused": True, "output_drained": True, "focus_acknowledgement": focus}
