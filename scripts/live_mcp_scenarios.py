"""Named acceptance cases using the real app's HTTP API and rendered debug controls."""
import csv
import base64
import ctypes
import io
import json
import math
import shutil
import socket
import subprocess
import time
from contextlib import ExitStack

from live_mcp_client import Client, wait_until
from live_mcp_native import DeniedScheduling, ScopedDebugPrivilege, OwnedProcess, available_cpus, listener_pid, pin_workload


def correction_denial(fixture, client, main, helper):
    """A denied write pauses after three refusals without losing readable CPU/RAM."""
    cpus = available_cpus()
    assert len(cpus) >= 2, 'This case requires two available CPUs'
    cpu, initial = cpus[-2], cpus[-1]
    group = client.mutate('create_group', {'name': 'Access test', 'cpus': [cpu]})['group_id']
    rule = {'name': 'Owned denied app', 'path': str(main), 'args': [],
            'tracked_process_names': [main.name, helper.name], 'manage_descendants': False,
            'autorun': False, 'priority': 'Normal'}
    rule_id = client.mutate('create_rule', {'group_id': group, 'rule': rule})['rule_id']
    children = []
    for path in [main, helper]:
        stop = fixture.root / (path.stem + '.stop')
        child = subprocess.Popen([str(path), str(stop)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        fixture.children.append((child, stop))
        children.append(child)
        pin_workload(child, initial)
    client.mutate('update_preferences', {'preferences': {'cpu_load_monitoring_enabled': True}})
    status = 'app.status.' + rule_id

    def measured(state):
        rows = controls(state)
        values = [rows.get('meter.' + scope, {}) for scope in ['rule.' + rule_id, 'group.' + group]]
        memory = [rows.get('memory.' + scope, {}) for scope in ['rule.' + rule_id, 'group.' + group]]
        return (all(v.get('instances') == 2 and v.get('measured') == 2
                    and v.get('cpu_percent') is not None and v['cpu_percent'] >= 0 for v in values)
                and values[0]['sample_unix_ms'] == values[1]['sample_unix_ms']
                and abs(values[0]['cpu_percent'] - values[1]['cpu_percent']) < .001
                and all(m.get('instances') == 2 and m.get('measured') == 2
                        and (m.get('private_bytes') or 0) > 0 for m in memory)
                and memory[0]['private_bytes'] == memory[1]['private_bytes'])

    wait_until('owned pair CPU/RAM', fixture.state, measured, seconds=25)
    with ExitStack() as undo:
        runner_debug = ScopedDebugPrivilege()
        undo.callback(runner_debug.restore)
        app_debug = ScopedDebugPrivilege(fixture.process)
        undo.callback(app_debug.restore)
        denial = DeniedScheduling(children[0])
        undo.callback(denial.restore)
        # Verify the intended native failure before claiming coverage from a GUI poll.
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
        kernel.OpenProcess.restype = ctypes.c_void_p
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        ctypes.set_last_error(0)
        opened = kernel.OpenProcess(0x200, False, children[0].pid)
        error = ctypes.get_last_error()
        if opened:
            kernel.CloseHandle(opened)
        assert not opened and error == 5, 'Owned DACL did not actually deny scheduling'
        client.mutate('update_preferences', {'preferences': {'process_monitoring_enabled': True}})
        wait_until('three denied automatic corrections', fixture.state,
                   lambda s: controls(s).get(status, {}).get('denials') == 3
                   and controls(s)[status]['status'] == 'Limited access', seconds=20)
        observed = time.monotonic()
        qualified = 0
        while time.monotonic() - observed < 7 or qualified < 3:
            assert time.monotonic() - observed < 15, 'Insufficient qualified observations of the denial pause'
            state = fixture.state()
            count = controls(state)[status]['denials']
            if count is not None:
                assert count == 3, 'A qualified denial counter changed during pause'
                qualified += 1
            assert measured(state), 'Denied writes lost readable rule/group telemetry'
            time.sleep(.5)
        for theme, index in [('dark', 2), ('light', 1)]:
            client.mutate('update_preferences', {'preferences': {'theme_index': index}})
            image = fixture.rpc({'action': 'screenshot'})
            assert image.get('image')
            (fixture.root / ('limited-access-' + theme + '.png')).write_bytes(base64.b64decode(image['image']))
    # Restored access alone must not silently resume the suspended write episode.
    restored = wait_until('qualified denial readback after DACL restoration', fixture.state,
                          lambda state: controls(state)[status]['denials'] is not None)
    assert controls(restored)[status]['denials'] == 3
    fixture.click('app.action.' + rule_id)
    wait_until('explicit Retry clears Limited access', fixture.state,
               lambda s: controls(s).get(status, {}).get('status') == 'Running · protected'
               and controls(s)[status]['denials'] == 0, seconds=10)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.GetProcessAffinityMask.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)]
    for child in children:
        process, system = ctypes.c_size_t(), ctypes.c_size_t()
        assert kernel.GetProcessAffinityMask(ctypes.c_void_p(int(child._handle)), ctypes.byref(process), ctypes.byref(system))
        assert process.value == 1 << cpu, 'Owned scheduling readback disagrees with saved group'
    readback = wait_until('complete CPU cut at native readback', fixture.state, measured)
    readback_cut = controls(readback)['meter.rule.' + rule_id]['sample_seconds']
    wait_until('fresh in-mask CPU/RAM recovers after Retry', fixture.state,
               lambda state: measured(state)
               and controls(state)['meter.rule.' + rule_id]['sample_seconds'] > readback_cut
               and controls(state)['meter.rule.' + rule_id]['cpu_percent'] > 0)
    client.mutate('update_preferences', {'preferences': {'process_monitoring_enabled': False}})
    return {'three_denials_paused': True, 'readable_cpu_ram_preserved': True,
            'retry_resumed': True, 'both_processes_native_readback': True, 'owned_dacl_restored': True}


def installed_read_only(fixture, client, aumid):
    """Explicit optional real-app observation; never enables enforcement or launches it."""
    assert client.tool('get_app_state')['preferences']['process_monitoring_enabled'] is False
    group = client.mutate('create_group', {'name': 'Read-only installed observation', 'cpus': available_cpus()})['group_id']
    rule = {'name': 'Read-only installed app', 'aumid': aumid, 'args': [],
            'tracked_process_names': [], 'manage_descendants': False, 'autorun': False, 'priority': 'Normal'}
    rule_id = client.mutate('create_rule', {'group_id': group, 'rule': rule})['rule_id']
    client.mutate('update_preferences', {'preferences': {'cpu_load_monitoring_enabled': True}})
    rows = wait_until('installed interactive processes discovered',
                      lambda: client.tool('read_cpu_current', {'limit': 256})['current_processes'],
                      lambda rows: any(p['rule_id'] == rule_id for p in rows), seconds=25)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.ProcessIdToSessionId.argtypes = [ctypes.c_uint32, ctypes.POINTER(ctypes.c_uint32)]
    session = ctypes.c_uint32()
    assert kernel.ProcessIdToSessionId(fixture.process.pid, ctypes.byref(session)) and session.value != 0
    members = [p for p in rows if p['rule_id'] == rule_id]
    for member in members:
        actual = ctypes.c_uint32()
        assert kernel.ProcessIdToSessionId(member['pid'], ctypes.byref(actual))
        assert actual.value == session.value, 'Automatic Installed membership admitted a service/foreign session'
    assert client.tool('get_app_state')['preferences']['process_monitoring_enabled'] is False
    assert json.loads(fixture.state_path.read_bytes())['process_monitoring_enabled'] is False
    return {'interactive_members': len(members), 'all_in_current_nonzero_session': True,
            'enforcement_remained_off': True, 'real_app_launch_or_mutation': False}


def controls(state):
    return {row["id"]: row["extra"] for row in state["controls"]}


def assert_capacity_cut(meter, cpus, pages):
    """Compare the visible meter with complete history at its exact original time."""
    assert meter['selected_cpu_count'] == len(cpus) == len(pages)
    loads = []
    for cpu, page in zip(cpus, pages):
        assert page['cpu'] == cpu, 'History returned a different logical CPU'
        row = next((row for row in page['rows'] if row['seconds'] == meter['sample_seconds']), None)
        assert row and row['unix_ms'] == meter['sample_unix_ms'], 'History must use the same original cut'
        assert row['delivery'] == 'completed' and row['load_quality'] == 'complete' and row['load'] is not None
        loads.append(row['load'])
    assert abs(meter['cpu_percent'] - sum(loads) / len(cpus)) < 0.01, 'Meter differs from selected-CPU scheduler work'


def select_owned_steam_process(fixture, process):
    target = 'steam.process.' + str(process.pid)
    def visible():
        assert process.poll() is None, 'Owned Steam workload exited during selection'
        rows = {row['id']: row for row in fixture.state()['controls']}
        if rows.get(target, {}).get('enabled'):
            return True
        if target not in rows:
            # The chooser is a request-driven snapshot, not a process watcher.
            fixture.click('steam.choose_process')
            fixture.click('steam.choose_process')
        else:
            rendered = [key for key, row in rows.items()
                        if key.startswith('steam.process.') and row['enabled']]
            if rendered:
                # Overlap successive views of the 160-point list instead of skipping rows.
                fixture.call('input', target=rendered[len(rendered) // 2], kind='scroll', delta=80)
        return False
    wait_until('visible owned Steam process', visible, bool)
    fixture.click(target)


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
    select_owned_steam_process(fixture, process)
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
    assert all(r['cpu_percent_basis'] == 'group_cpu_capacity' for r in cpu)
    assert cpu[0]['sample_seconds'] is not None and len({r['sample_seconds'] for r in cpu}) == 1, 'Mixed scheduler intervals'
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
    # The pointer-only debug adapter exercises the real GUI form; creation and
    # failed saves have deterministic state-level coverage without a text RPC.
    baseline = client.tool('list_groups')['rows']
    selected = next(row for row in baseline if row['group_id'] == second)
    client.mutate('update_group', {'group_id': second, 'name': selected['name'],
                  'cpus': [cpu], 'run_all': True, 'hidden': True})
    revision = client.tool('get_app_state')['configuration_revision']
    fixture.click('group.edit.' + second)
    fixture.click('group.form.run_all')
    fixture.click('group.form.save')
    wait_until('GUI group edit durable receipt', lambda: client.tool('get_app_state'),
               lambda state: state['configuration_revision'] != revision)
    wait_until('GUI group form closes after its own save', fixture.state,
               lambda state: 'group.form.close' not in controls(state))
    edited = next(row for row in client.tool('list_groups')['rows'] if row['group_id'] == second)
    assert edited['name'] == selected['name'] and edited['cpus'] == [cpu]
    assert edited['hidden'] is True and edited['run_all'] is False, 'GUI edit changed unrelated saved fields'
    assert client.tool('list_rules', {'group_id': second})['rows'][0]['rule_id'] == rule_id
    assert next(row for row in client.tool('list_groups')['rows'] if row['group_id'] == first) == next(row for row in baseline if row['group_id'] == first)
    # Restore visibility so subsequent telemetry assertions inspect actual rows.
    client.mutate('update_group', {'group_id': second, 'name': edited['name'],
                  'cpus': edited['cpus'], 'run_all': edited['run_all'], 'hidden': False})
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
    try:
        wait_until("root/helper measured", fixture.state, measured, seconds=25)
    except AssertionError:
        current = fixture.state()
        scopes = ['group.' + group, 'rule.' + rule_id, 'rule.' + shared,
                  'group.' + peer_group, 'rule.' + peer]
        # Only known synthetic metric controls, never a process inventory or credentials.
        view = controls(current)
        (fixture.root / 'capacity-failure.json').write_text(json.dumps({
            'meters': {scope: view.get('meter.' + scope) for scope in scopes},
            'memory': {scope: view.get('memory.' + scope) for scope in scopes},
            'collector': {key: (current.get('graph') or {}).get(key) for key in ['error', 'detail_error']},
        }, indent=2), encoding='utf-8')
        raise
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
    # Actual discovery churn in a different group must not blank stable meters.
    churn_path = fixture.root / ('owned-churn-' + main.name)
    shutil.copy2(main, churn_path)
    churn_rule = client.mutate('create_rule', {'group_id': peer_group, 'rule': {
        'name': 'Owned unrelated churn', 'path': str(churn_path), 'args': [],
        'tracked_process_names': [churn_path.name], 'manage_descendants': False,
        'autorun': False, 'priority': 'Normal'}})['rule_id']
    stable_cuts = set()
    churn_cpu = next(c for c in available_cpus() if c != cpu)

    def observe_churn(predicate):
        deadline = time.monotonic() + 15
        while True:
            rows = controls(fixture.state())
            assert_unions(rows, group, rule_id, shared)
            stable_cuts.add(rows['meter.rule.' + rule_id]['sample_seconds'])
            if predicate(rows):
                return
            assert time.monotonic() < deadline, 'Unrelated churn was not observed'
            time.sleep(.2)

    for cycle in range(2):
        stop = fixture.root / ('churn-' + str(cycle) + '.stop')
        child = subprocess.Popen([str(churn_path), str(stop)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        fixture.children.append((child, stop))
        pin_workload(child, churn_cpu)
        observe_churn(lambda rows: rows.get('meter.rule.' + churn_rule, {}).get('instances') == 1)
        cuts_before = len(stable_cuts)
        observe_churn(lambda rows: len(stable_cuts) >= cuts_before + 2)
        stop.write_text('stop', encoding='utf-8')
        assert child.wait(timeout=5) == 0
        observe_churn(lambda rows: rows.get('meter.rule.' + churn_rule, {}).get('instances') == 0)
    client.mutate('delete_rule', {'group_id': peer_group, 'rule_id': churn_rule})
    wait_until('peer union after churn cleanup', fixture.state, measured, seconds=20)
    # The modal hides the main list. Read its original-time history through HTTP
    # while the meter is visible, then exercise graph navigation separately.
    def matches_history(state, cpus):
        rows = controls(state)
        meter = rows.get('meter.rule.' + rule_id, {})
        if (meter.get('sample_seconds') is None or meter.get('measured') != 2
                or meter.get('selected_cpu_count') != len(cpus)):
            return False
        pages = []
        for selected in cpus:
            query = {'source': 'rule', 'group_id': group, 'rule_id': rule_id, 'cpu': selected, 'limit': 256}
            page = client.tool('read_cpu_history', query)
            if page['total_retained'] > 256:
                page = client.tool('read_cpu_history', dict(query, offset=page['total_retained'] - 256))
            row = next((r for r in page['rows'] if r['seconds'] == meter['sample_seconds']), None)
            if row is None or row['delivery'] != 'completed' or row['load'] is None:
                return False
            pages.append(page)
        assert_capacity_cut(meter, cpus, pages)
        assert_unions(rows, group, rule_id, shared)
        return True
    wait_until('one-CPU capacity equals original history', fixture.state,
               lambda state: matches_history(state, [cpu]), seconds=20)
    spare = next(c for c in available_cpus() if c != cpu)
    saved_group = next(row for row in client.tool('list_groups')['rows'] if row['group_id'] == group)
    def set_mask(cpus):
        client.mutate('update_group', {'group_id': group, 'name': saved_group['name'],
                      'run_all': saved_group['run_all'], 'hidden': saved_group['hidden'], 'cpus': cpus})
    set_mask([cpu, spare])
    wait_until('two-CPU capacity uses the full selected denominator', fixture.state,
               lambda state: matches_history(state, [cpu, spare]), seconds=20)
    previous_cut = controls(fixture.state())['meter.rule.' + rule_id]['sample_seconds']
    set_mask([spare])
    def outside_excluded(state):
        rows = controls(state)
        meter = rows.get('meter.rule.' + rule_id, {})
        if (meter.get('sample_seconds') is None or meter['sample_seconds'] <= previous_cut
                or meter.get('measured') != 2 or meter.get('cpu_percent') != 0):
            return False
        assert_unions(rows, group, rule_id, shared)
        assert rows['meter.group.' + group]['cpu_percent'] == 0
        assert rows['meter.group.' + peer_group]['cpu_percent'] > 0, 'The owned children must still be busy outside this mask'
        return True
    wait_until('busy outside-mask work is excluded', fixture.state, outside_excluded, seconds=20)
    set_mask([cpu])
    wait_until('capacity recovers after mask restoration', fixture.state,
               lambda state: matches_history(state, [cpu]), seconds=20)
    fixture.click('monitor.open')
    fixture.click('monitor.tab.processes')
    peer_name = next(row['name'] for row in client.tool('list_groups')['rows'] if row['group_id'] == peer_group)
    wait_until('scoped duplicate launch rules remain in the Process monitor roster', fixture.state,
               lambda state: all(any(row['pid'] == child.pid and row['group'] == peer_name
                                    for row in controls(state).get('table.tracked', {}).get('entries', []))
                                 for child, _ in workloads))
    fixture.click('monitor.close')
    fixture.call('open_graph', group_id=group, rule_id=rule_id)
    wait_until('initial rendered app metadata', fixture.state,
               lambda s: controls(s).get('graph.metadata', {}).get('name') == 'Renamed owned rule')
    fixture.call('select_cpu', cpu=cpu)
    original = next(row for row in reversed(client.tool('read_cpu_history', {'source': 'system', 'cpu': cpu, 'limit': 256})['rows']) if row['delivery'] == 'completed' and row['load'] is not None)
    saved_rule = next(row['app'] for row in client.tool('list_rules', {'group_id': group})['rows']
                      if row['rule_id'] == rule_id)
    edited = {'name': 'Live updated app', 'path': str(main), 'args': saved_rule['args'],
              'tracked_process_names': [helper.name, main.name], 'manage_descendants': True,
              'autorun': False, 'priority': saved_rule['priority']}
    client.mutate('update_rule', {'group_id': group, 'rule_id': rule_id, 'rule': edited})
    def refreshed(state):
        rendered = controls(state).get('graph.metadata', {})
        return (rendered.get('name') == edited['name']
                and rendered.get('scope') == 'Tracks: ' + ', '.join(edited['tracked_process_names']) + ' + descendants'
                and rendered.get('cpus') == [cpu]
                and state['graph']['selected_cpu'] == cpu)
    wait_until('committed metadata updates without closing the graph', fixture.state, refreshed)
    retained = client.tool('read_cpu_history', {'source': 'system', 'cpu': cpu, 'limit': 256})['rows']
    assert any(row['seconds'] == original['seconds'] and row['load'] == original['load'] and row['unix_ms'] == original['unix_ms'] for row in retained), 'Metadata edit erased numeric history'
    fixture.call('close_graph')
    fixture.call("open_graph", group_id=group)
    graph = fixture.state()["graph"]
    renamed_group = saved_group['name'] + ' updated'
    client.mutate('update_group', {'group_id': group, 'name': renamed_group,
                  'run_all': saved_group['run_all'], 'hidden': saved_group['hidden'], 'cpus': [cpu]})
    wait_until('committed group graph title', fixture.state,
               lambda s: controls(s).get('graph.metadata', {}).get('name') == renamed_group)
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
    return {"fresh_union_samples": len(seen), "unrelated_churn_cycles": 2, "stable_churn_cuts": len(stable_cuts), "graph_opened": bool(graph), "cpu": cpu,
            'selected_capacity_matches_history': True, 'outside_mask_excluded': True, 'mask_edits_verified': True, 'open_graph_metadata_updated': True, 'duplicate_roster_verified': True}


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
