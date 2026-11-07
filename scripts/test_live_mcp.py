"""Critical harness guards; these tests do not launch an app or touch Windows."""
import json
import unittest
from pathlib import Path
from unittest.mock import patch

from live_mcp_client import decode_reply, validate_endpoint
from live_mcp_fixture import Fixture, FixtureCleanupError, synthetic_state, validate_state
from live_mcp_scenarios import assert_capacity_cut, assert_unions, history, select_owned_steam_process
from run_live_mcp import owned_workload_paths, retire_fixture


class HarnessGuards(unittest.TestCase):
    def test_history_checks_latest_completed_work_instead_of_a_startup_interval(self):
        class Client:
            contributor_times = []
            def tool(self, name, arguments=None, error=None):
                arguments = arguments or {}
                if name == 'read_cpu_history':
                    if error:
                        return {'code': error}
                    rows = [{'seconds': 10, 'delivery': 'completed', 'load': 0},
                            {'seconds': 11, 'delivery': 'completed', 'load': 25},
                            {'seconds': 12, 'delivery': 'pending', 'load': None}]
                    return {'total_retained': 3, 'rows': rows[:arguments['limit']]}
                if name == 'read_cpu_contributors':
                    self.contributor_times.append(arguments['seconds'])
                    return {'rows': [] if arguments['seconds'] == 10 else [
                        {'name': 'helper.exe', 'load': 25, 'historical_lifetime_id': '7'}]}
                if name == 'export_cpu_history':
                    text = ('time_utc,elapsed_seconds,record_type,process,cpu_percent,historical_lifetime_id\r\n'
                            'utc,11,cpu,,25,\r\nutc,11,process,helper.exe,25,7\r\n')
                    return {'export_id': 'owned', 'status': 'ready', 'text': text,
                            'after': '0', 'next_after': str(len(text.encode())), 'eof': True}
                raise AssertionError('Unexpected test tool: ' + name)
        class Fixture:
            def call(self, action, **arguments):
                return {'satisfied': True}
        client = Client()
        history(Fixture(), client, 0, Path('helper.exe'))
        self.assertEqual(client.contributor_times, [11],
                         'A pending row or empty startup interval cannot stand in for measured owned work')

    def test_cleanup_retry_does_not_reuse_a_closed_owned_process_handle(self):
        class Child:
            closed = False
            waits = 0
            def wait(self, timeout):
                if self.closed:
                    raise AssertionError('Owned handle already closed')
                self.waits += 1
                return 0
            def close(self):
                self.closed = True
        class Stop:
            def write_text(self, *args, **kwargs):
                pass
        child = Child()
        fixture = Fixture.__new__(Fixture)
        fixture.root, fixture.process, fixture.streams = Path('synthetic-fixture'), None, []
        fixture.children = [(child, Stop())]
        def failed_close():
            raise AssertionError('Synthetic app shutdown failure')
        fixture.close = failed_close
        with patch.object(Path, 'unlink'), patch.object(Path, 'exists', return_value=False):
            with self.assertRaises(FixtureCleanupError):
                fixture.cleanup()
            fixture.close = lambda: None
            fixture.cleanup()
        self.assertTrue(child.closed)
        self.assertEqual(child.waits, 1)

    def test_forced_cleanup_failure_survives_a_successful_final_retry(self):
        class FailedOnce:
            root = Path('synthetic-fixture')
            calls = 0
            def cleanup(self):
                self.calls += 1
                if self.calls == 1:
                    raise FixtureCleanupError(['workload required forced cleanup'])
        fixture = FailedOnce()
        fixtures, failures = [fixture], []
        with self.assertRaises(FixtureCleanupError):
            retire_fixture(fixture, fixtures, failures)
        retire_fixture(fixture, fixtures, failures)
        self.assertFalse(fixtures)
        self.assertEqual(len(failures), 1, 'Successful retry must not turn forced cleanup into a pass')
        self.assertEqual(failures[0]['codes'], ['workload required forced cleanup'])

    def test_capacity_oracle_requires_original_time_complete_selected_cpus(self):
        meter = {'sample_seconds': 5.0, 'sample_unix_ms': '5000',
                 'selected_cpu_count': 2, 'cpu_percent': 30.0}
        def page(cpu, load, seconds=5.0):
            return {'cpu': cpu, 'rows': [{'seconds': seconds, 'unix_ms': '5000',
                    'delivery': 'completed', 'load_quality': 'complete', 'load': load}]}
        assert_capacity_cut(meter, [2, 3], [page(2, 60.0), page(3, 0.0)])
        for pages in [[page(2, 60.0, 4.0), page(3, 0.0)],
                      [page(2, 60.0), page(3, None)],
                      [page(2, 60.0), page(2, 0.0)],
                      [page(2, 30.0), page(3, 0.0)]]:
            with self.assertRaises(AssertionError):
                assert_capacity_cut(meter, [2, 3], pages)

    def test_owned_steam_choice_scrolls_before_clicking_a_clipped_row(self):
        class Picker:
            offset = 0
            clicked = False
            def state(self):
                return {'controls': [
                    {'id': 'steam.process.' + str(42 if index == 9 else index + 1),
                     'enabled': index * 20 < self.offset + 160 and (index + 1) * 20 > self.offset}
                    for index in range(30)]}
            def call(self, action, **arguments):
                current = {row['id']: row for row in self.state()['controls']}
                if (action != 'input' or not current[arguments['target']]['enabled']
                        or arguments.get('kind') != 'scroll' or arguments.get('delta', 0) <= 0):
                    raise AssertionError('The debug input API needs positive delta to reveal a lower row')
                self.offset = min(440, self.offset + arguments['delta'])
                if self.offset >= 200:
                    raise AssertionError('Scrolling skipped the owned row completely')
            def click(self, target):
                current = {row['id']: row for row in self.state()['controls']}
                if not current[target]['enabled']:
                    raise AssertionError('Owned row is clipped, not ready for a click')
                self.clicked = target
        class Child:
            pid = 42
            def poll(self):
                return None
        picker = Picker()
        select_owned_steam_process(picker, Child())
        self.assertEqual(picker.clicked, 'steam.process.42')

    def test_concurrent_enforcement_workloads_have_disjoint_basenames(self):
        first = owned_workload_paths(Path('first-run'), 'session-one', 'denied', 'readable')
        second = owned_workload_paths(Path('second-run'), 'session-two', 'denied', 'readable')
        self.assertTrue({p.name.lower() for p in first}.isdisjoint(p.name.lower() for p in second),
                        'Explicit name matching is global; separate directories do not isolate enforcement')

    def test_reply_requires_the_matching_id_and_bounds_sse(self):
        reply = {"jsonrpc": "2.0", "id": 7, "result": {"ok": True}}
        for content_type, payload in [
            ("application/json", json.dumps(reply).encode()),
            ("text/event-stream", ("event: message\ndata: " + json.dumps(reply) + "\n\n").encode()),
        ]:
            self.assertEqual(decode_reply(content_type, payload, 7), {"ok": True})
            with self.assertRaises(ValueError):
                decode_reply(content_type, payload, 8)
        with self.assertRaises(ValueError):
            decode_reply("text/event-stream", b"data: {}\n\n", 7)
        with self.assertRaises(ValueError):
            decode_reply("application/json", b" " * (2 * 1024 * 1024 + 1), 7)

    def test_connection_never_sends_a_key_outside_exact_loopback(self):
        validate_endpoint("http://127.0.0.1:43000/mcp", "a" * 64)
        for url in ["http://example.com/mcp", "http://127.0.0.1:43000/other",
                    "http://user@127.0.0.1:43000/mcp", "http://127.0.0.1:43000/mcp?x=1"]:
            with self.assertRaises(ValueError):
                validate_endpoint(url, "a" * 64)

    def test_synthetic_default_cannot_autorun_or_apply_a_tweak(self):
        state = synthetic_state(Path("owned-fixture"), version=13)
        validate_state(state)
        self.assertEqual(state["version"], 13)
        self.assertFalse(state["process_monitoring_enabled"])
        self.assertFalse(state["cpu_load_monitoring_enabled"])
        self.assertNotIn("tuning", state)
        for unsafe in [dict(state, tuning={"enabled": True}),
                       dict(state, groups=[{"programs": [{"autorun": True}]}])]:
            with self.assertRaises(ValueError):
                validate_state(unsafe)

    def test_union_oracle_rejects_double_count_and_mixed_samples(self):
        controls = {}
        for kind, value in [("meter", 2.0), ("memory", 100)]:
            field = "cpu_percent" if kind == "meter" else "private_bytes"
            for scope in ["group.g", "rule.a", "rule.b"]:
                controls[f"{kind}.{scope}"] = {field: value, "sample_unix_ms": "1", 'sample_seconds': 1.0, 'cpu_percent_basis': 'group_cpu_capacity'}
        assert_unions(controls, "g", "a", "b")
        controls["meter.group.g"]["cpu_percent"] = 4.0
        with self.assertRaises(AssertionError):
            assert_unions(controls, "g", "a", "b")
        controls["meter.group.g"]["cpu_percent"] = 2.0
        controls["meter.rule.b"]["sample_unix_ms"] = "2"
        with self.assertRaises(AssertionError):
            assert_unions(controls, "g", "a", "b")


if __name__ == "__main__":
    unittest.main()
