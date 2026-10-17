"""Standard-library tests for the local stdio MCP adapter; no GUI or user files."""
import unittest
import urllib.error
import io
from unittest import mock
import cpu_affinity_mcp as server


class ProtocolTests(unittest.TestCase):
    def test_stdio_reply_is_utf8_even_with_a_windows_legacy_output_encoding(self):
        request = {"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"cpu_state", "arguments":{}}}
        source = io.TextIOWrapper(io.BytesIO((server.json.dumps(request) + "\n").encode("utf-8")), encoding="utf-8")
        destination = io.BytesIO()
        stdout = io.TextIOWrapper(destination, encoding="cp1252")
        with mock.patch.object(server.sys, "stdin", source), mock.patch.object(server.sys, "stdout", stdout), mock.patch.object(server.sys, "argv", ["adapter", "--session", "unused"]), mock.patch.object(server, "rpc_from_ticket", return_value=lambda _: {"ok":True,"data":{"title":"… Ядро"}}):
            server.main()
        reply = server.json.loads(destination.getvalue().decode("utf-8"))
        self.assertEqual(server.json.loads(reply["result"]["content"][0]["text"])["data"]["title"], "… Ядро")

    def test_three_diagnostic_tools_have_bounded_arguments_and_exact_actions(self):
        tools={s[0]:s for s in server.SPECS}
        for name,action,args in [('cpu_performance','performance',{'seconds':10}),('cpu_quality','quality',{'limit':20}),('cpu_wait','wait',{'condition':'new_sample','timeout_ms':1000})]:
            self.assertIn(name,tools)
            calls=[]
            result=server.dispatch({'id':9,'method':'tools/call','params':{'name':name,'arguments':args}},lambda body:calls.append(body) or {'ok':True})
            self.assertFalse(result['result']['isError'])
            self.assertEqual(calls,[{'action':action,**args}])
        for name,args in [('cpu_performance',{'seconds':61}),('cpu_wait',{'condition':'new_sample','timeout_ms':6001}),('cpu_wait',{'condition':'shell'})]:
            result=server.dispatch({'id':10,'method':'tools/call','params':{'name':name,'arguments':args}},lambda _:self.fail('invalid input reached app'))
            self.assertTrue(result['result']['isError'])
    def test_malformed_rpc_params_do_not_terminate_the_adapter(self):
        for params in [[], 42, "bad"]:
            result = server.dispatch({"jsonrpc":"2.0", "id":7, "method":"initialize", "params":params}, None)
            self.assertEqual(result["error"]["code"], -32602)

    def test_redirects_are_rejected_instead_of_forwarding_the_session_key(self):
        with self.assertRaises(urllib.error.HTTPError):
            server.NoRedirect().redirect_request(None, None, 302, "redirect", {}, "http://foreign.example/rpc")

    def test_initialize_and_tool_listing_do_not_need_a_running_app(self):
        init = server.dispatch({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}, None)
        self.assertEqual(init["result"]["protocolVersion"], "2025-06-18")
        tools = server.dispatch({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}, None)["result"]["tools"]
        self.assertTrue(tools)
        self.assertTrue(all(t["inputSchema"]["additionalProperties"] is False for t in tools))

    def test_tool_uses_only_declared_action_and_keeps_images_as_mcp_images(self):
        calls = []
        def rpc(body):
            calls.append(body)
            return {"ok": True, "data": {"frame": 2}}
        result = server.dispatch({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "cpu_state", "arguments": {}}}, rpc)
        self.assertFalse(result["result"]["isError"])
        self.assertEqual(calls, [{"action": "state"}])
        bad = server.dispatch({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "cpu_state", "arguments": {"action": "shell"}}}, rpc)
        self.assertTrue(bad["result"]["isError"])
        self.assertEqual(len(calls), 1)

    def test_notifications_have_no_response_and_unknown_methods_are_errors(self):
        self.assertIsNone(server.dispatch({"jsonrpc": "2.0", "method": "notifications/initialized"}, None))
        result = server.dispatch({"jsonrpc": "2.0", "id": 5, "method": "shell"}, None)
        self.assertEqual(result["error"]["code"], -32601)

    def test_session_ticket_cannot_redirect_to_network_or_non_rpc_paths(self):
        for url in ["https://foreign.example/rpc", "http://127.0.0.1.evil/rpc", "http://127.0.0.1:1234/other"]:
            with self.assertRaises(ValueError):
                server.validate_ticket({"url": url, "token": "a" * 64})
        self.assertTrue(server.validate_ticket({"url": "http://127.0.0.1:1234/rpc", "token": "a" * 64}))


if __name__ == "__main__":
    unittest.main()
