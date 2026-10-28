"""Critical harness guards; these tests do not launch an app or touch Windows."""
import json
import unittest
from pathlib import Path

from live_mcp_client import decode_reply, validate_endpoint
from live_mcp_fixture import synthetic_state, validate_state
from live_mcp_scenarios import assert_unions


class HarnessGuards(unittest.TestCase):
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
                controls[f"{kind}.{scope}"] = {field: value, "sample_unix_ms": "1"}
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
