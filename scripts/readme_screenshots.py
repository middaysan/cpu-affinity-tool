"""Repeatable README scenes using the existing live MCP fixture and real PNG frames."""
import base64
import datetime
import hashlib
import json
import pathlib
import shutil
import struct
import subprocess
import time
import tomllib

from live_mcp_client import wait_until
from live_mcp_fixture import Fixture, assert_diagnostic_exe, synthetic_state
from live_mcp_native import available_cpus
from live_mcp_scenarios import connect, controls

REPO = pathlib.Path(__file__).resolve().parent.parent
CONTRACT = REPO / 'scripts/fixtures/readme-screenshots.json'
PNG_SIGNATURE = b'\x89PNG\r\n\x1a\n'
PNG_END = b'\x00\x00\x00\x00IEND\xaeB`\x82'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def png_size(data):
    if (not 45 <= len(data) <= 10 * 1024 * 1024 or not data.startswith(PNG_SIGNATURE)
            or data[12:16] != b'IHDR' or not data.endswith(PNG_END)):
        raise ValueError('Expected a complete bounded PNG frame')
    width, height = struct.unpack_from('>II', data, 16)
    if not 320 <= width <= 4096 or not 400 <= height <= 4096:
        raise ValueError('Screenshot dimensions are outside the capture budget')
    return width, height


def verify_capture(root, report):
    config = json.loads(CONTRACT.read_bytes())
    if (report.get('schema') != 1 or report.get('status') != 'pass' or report.get('cleanup') != 'pass'
            or report.get('contract_sha256') != digest(CONTRACT)):
        raise ValueError('Only a complete capture with successful cleanup can be published')
    expected = {scene['id']: scene['file'] for scene in config['scenes']}
    if set(report.get('captures', {})) != set(expected):
        raise ValueError('The required screenshot set is incomplete')
    widths = set()
    for scene, filename in expected.items():
        record = report['captures'][scene]
        if record.get('file') != filename:
            raise ValueError('Screenshot paths must match the fixed asset whitelist')
        path = root / filename
        if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
            raise ValueError('Screenshot escaped its capture directory')
        if digest(path) != record.get('sha256'):
            raise ValueError('Screenshot bytes differ from the native capture')
        size = png_size(path.read_bytes())
        if size != (record.get('width'), record.get('height')):
            raise ValueError('Screenshot dimensions differ from capture metadata')
        widths.add(size[0])
    if len(widths) != 1:
        raise ValueError('Capture all scenes at the same app width and scale')


def ui_inputs():
    paths = {path for pattern in json.loads(CONTRACT.read_bytes())['ui_inputs']
             for path in REPO.glob(pattern) if path.is_file()}
    return {path.relative_to(REPO).as_posix(): digest(path) for path in sorted(paths)}


def capture(exe, workload, output):
    assert_diagnostic_exe(exe)
    output.mkdir(parents=True, exist_ok=False)
    images = output / 'screenshots'
    images.mkdir()
    report = {'schema': 1, 'status': 'running', 'cleanup': 'pending',
              'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
              'package_version': tomllib.loads((REPO / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version'],
              'artifact_sha256': digest(exe), 'artifact_kind': 'diagnostic-dev',
              'captured_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'contract_sha256': digest(CONTRACT), 'ui_inputs': ui_inputs(), 'captures': {},
              'conditions': {'data': 'synthetic rules and owned workloads', 'charts': 'measured, not injected',
                             'native_tweaks_applied': False, 'credentials_in_assets': False}}
    fixture = None

    def progress(phase):
        (output / 'progress.json').write_text(json.dumps({'phase': phase, 'captured': list(report['captures'])}), encoding='utf-8')

    def snapshot(scene, neutral='nav.groups', x=0.05, y=0.1):
        progress(scene)
        fixture.call('input', target=neutral, kind='move', x=x, y=y)
        assert fixture.call('wait', condition='ui_settled', timeout_ms=6000)['satisfied']
        # Let the finite authenticated-command pulse and hover animation settle.
        time.sleep(0.25)
        frame = fixture.rpc({'action': 'screenshot'})
        if not frame.get('ok') or not frame.get('image'):
            raise AssertionError('Native frame capture failed')
        data = base64.b64decode(frame['image'], validate=True)
        width, height = png_size(data)
        spec = next(s for s in json.loads(CONTRACT.read_bytes())['scenes'] if s['id'] == scene)
        path = images / spec['file']
        path.write_bytes(data)
        report['captures'][scene] = {'file': spec['file'], 'sha256': digest(path), 'width': width,
                                    'height': height, 'frame': frame.get('frame'), 'theme': spec['theme'],
                                    'label': spec['label']}

    try:
        progress('startup')
        fixture = Fixture(exe, output / 'app', synthetic_state(output))
        fixture.start()
        client = connect(fixture)
        report['pixels_per_point'] = fixture.state()['viewport']['pixels_per_point']
        names = ['DemoApp.exe', 'DemoHelper.exe', 'BackgroundWorker.exe', 'OptionalApp.exe']
        offset = 0
        for _ in range(32):
            inventory = client.tool('list_processes', {'offset': offset, 'limit': 256})
            if any(row['name'].lower() in {name.lower() for name in names} for row in inventory['rows']):
                raise AssertionError('A demonstration process name is already in use; no enforcement started')
            offset = inventory.get('next_offset')
            if offset is None:
                break
        else:
            raise AssertionError('Cannot bound demonstration-name preflight')
        cpus = available_cpus()
        foreground = cpus[:min(4, len(cpus))]
        background = cpus[-min(4, len(cpus)):]
        first = client.mutate('create_group', {'name': 'Performance', 'cpus': foreground})['group_id']
        second = client.mutate('create_group', {'name': 'Background', 'cpus': background})['group_id']
        paths = {name: output / name for name in names}
        for path in paths.values():
            shutil.copy2(workload, path)
        for group, name, filename, tracked in [
            (first, 'Sample application', names[0], names[:2]),
            (first, 'Another application', names[3], [names[3]]),
            (second, 'Background worker', names[2], [names[2]])
        ]:
            client.mutate('create_rule', {'group_id': group, 'rule': {
                'name': name, 'path': str(paths[filename]), 'args': [], 'tracked_process_names': tracked,
                'priority': 'Normal', 'autorun': False, 'manage_descendants': False}})
        client.mutate('update_preferences', {'preferences': {'process_monitoring_enabled': True,
                      'cpu_load_monitoring_enabled': True, 'cpu_load_history_seconds': 30}})
        for filename in names[:3]:
            stop = output / (filename + '.stop')
            child = subprocess.Popen([str(paths[filename]), str(stop)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            fixture.children.append((child, stop))
        progress('recording measured demonstration history')
        target_cpu = foreground[0]
        def ready():
            current = controls(fixture.state())
            group = current.get('meter.group.' + first, {})
            if group.get('instances') != 2 or group.get('measured') != 2 or (group.get('cpu_percent') or 0) <= 0:
                return False
            history = client.tool('read_cpu_history', {'source': 'system', 'cpu': target_cpu, 'limit': 60})
            return sum(row['delivery'] == 'completed' for row in history.get('rows', [])) >= 18
        wait_until('measured history and complete demo membership', ready, bool, seconds=65)
        snapshot('main-dark')
        fixture.click('group.edit.' + first)
        snapshot('topology', 'group.form.close')
        fixture.click('group.form.close')
        fixture.call('open_graph', group_id=first)
        fixture.call('graph_options', all_cpus=False, tracked_only=True, view='graphs')
        state = fixture.state()
        tiles = (state.get('rendered_graph') or {}).get('tiles', [])
        if tiles:
            target_cpu = max(tiles, key=lambda tile: tile.get('load') or 0)['cpu']
        fixture.call('select_cpu', cpu=target_cpu)
        wait_until('visible original graph data', fixture.state,
                   lambda s: (s.get('rendered_graph') or {}).get('series_points', 0) >= 18)
        snapshot('cpu-history', 'graph.close')
        fixture.click('view.processes')
        wait_until('demo process table', lambda: fixture.call('table', kind='processes'),
                   lambda table: table.get('total', 0) >= 2)
        snapshot('processes', 'graph.close')
        fixture.click('view.graphs')
        fixture.call('input', target='chart.plot', kind='right_click', x=0.7, y=0.45)
        snapshot('history-export', 'graph.export.png')
        fixture.call('close_graph')
        fixture.click('nav.tweaks')
        wait_until('read-only tweak cards', fixture.state,
                   lambda s: 'tweaks.body' in controls(s) and bool(s['tweaks']['cards'])
                   and not s['tweaks']['operation_error'])
        assert not fixture.state()['tweaks']['has_changes']
        snapshot('system-tweaks', 'nav.tweaks')
        fixture.click('nav.groups')
        fixture.click('mcp.status')
        snapshot('mcp-connection', 'mcp.connection.close')
        fixture.click('mcp.connection.close')
        fixture.click('settings.open')
        fixture.click('settings.theme.1')
        fixture.click('settings.open')
        snapshot('main-light')
        report['status'] = 'pass'
    except Exception as error:
        report.update(status='fail', failure_type=type(error).__name__)
        raise
    finally:
        try:
            if fixture:
                fixture.cleanup()
            report['cleanup'] = 'pass'
        except Exception:
            report.update(status='fail', cleanup='fail')
            raise
        finally:
            (images / 'capture.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
            progress('finished: ' + report['status'])
    verify_capture(images, report)
    print(json.dumps({'status': report['status'], 'cleanup': report['cleanup'], 'directory': str(images),
                      'scenes': list(report['captures'])}))
