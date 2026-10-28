"""Synthetic-only fixtures and exact child ownership. No user state is imported."""
import ctypes
import hashlib
import json
import os
import pathlib
import secrets
import shutil
import subprocess

from cpu_affinity_mcp import rpc_from_ticket
from live_mcp_client import wait_until


def synthetic_state(root, version=14):
    return {"version": version, "groups": [], "cpu_schema": {"model": "", "clusters": []}, "theme_index": 2,
            "process_monitoring_enabled": False, "cpu_load_monitoring_enabled": False,
            "cpu_load_history_seconds": 30, "start_minimized": False,
            "windows_event_log_diagnostics_enabled": False}


def validate_state(state):
    if ("tuning" in state or state.get("process_monitoring_enabled", True)
            or state.get("cpu_load_monitoring_enabled", True)
            or state.get("start_minimized", True)
            or any(app.get("autorun", True) for group in state.get("groups", [])
                   for app in group.get("programs", []))):
        raise ValueError("Synthetic startup refuses tuning, autorun, enforcement or hidden activation")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def assert_diagnostic_exe(path):
    # Read the actual PE manifest before launch: never accidentally request UAC for a shipping EXE.
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint32]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = ctypes.c_uint32
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    handle = kernel.LoadLibraryExW(str(path), None, 0x22)
    if not handle:
        raise ValueError("Cannot read the selected executable")
    try:
        resource = kernel.FindResourceW(handle, ctypes.c_void_p(1), ctypes.c_void_p(24))
        if resource:
            size = kernel.SizeofResource(handle, resource)
            data = ctypes.string_at(kernel.LockResource(kernel.LoadResource(handle, resource)), size)
            if b"requireAdministrator" in data or b"highestAvailable" in data:
                raise ValueError("Live diagnostic scenarios refuse an elevated shipping manifest")
    finally:
        kernel.FreeLibrary(handle)


class Fixture:
    def __init__(self, exe, root, state=None):
        state = state or synthetic_state(root)
        validate_state(state)
        self.root = root.resolve()
        self.root.mkdir(parents=True, exist_ok=False)
        self.exe = self.root / "owned-app.exe"
        shutil.copy2(exe, self.exe)
        self.state_path = self.root / "state.json"
        self.state_path.write_text(json.dumps(state), encoding="utf-8")
        (self.root / ".cpu-affinity-debug-isolated").write_text("debug-only", encoding="utf-8")
        self.children = []
        self.process = None
        self.rpc = None
        self.streams = []

    def start(self):
        ticket = self.root / "session.json"
        ticket.unlink(missing_ok=True)
        env = os.environ.copy()
        for key in list(env):
            if key.startswith("CPU_AFFINITY_"):
                del env[key]
        env.update(LOCALAPPDATA=str(self.root / "local-data"),
                   CPU_AFFINITY_MCP_TEST_DIR=str(self.root / "a"),
                   CPU_AFFINITY_TOOL_TEST_CRASH_REPORT_DIR=str(self.root / "crash-reports"),
                   CPU_AFFINITY_MCP_SESSION=str(ticket), CPU_AFFINITY_MCP_TOKEN=secrets.token_hex(32))
        self.streams = [(self.root / name).open("w", encoding="utf-8") for name in ["startup.out", "startup.err"]]
        self.process = subprocess.Popen([str(self.exe)], cwd=self.root, env=env,
                                        stdout=self.streams[0], stderr=self.streams[1])
        def ready():
            assert self.process.poll() is None, "Diagnostic app exited before readiness"
            if not ticket.exists():
                return False
            data = json.loads(ticket.read_text(encoding="utf-8-sig"))
            assert data["pid"] == self.process.pid, "Debug ticket belongs to a different process"
            return True
        wait_until("diagnostic readiness", ready, bool, seconds=30)
        self.rpc = rpc_from_ticket(ticket)
        self.state()
        return self

    def call(self, action, **arguments):
        reply = self.rpc(dict(action=action, **arguments))
        assert reply.get("ok"), "Diagnostic action failed: " + action
        return reply.get("data", reply)

    def state(self):
        return self.call("state")

    def click(self, control):
        wait_until(control, self.state, lambda state: any(c["id"] == control and c["enabled"] for c in state["controls"]))
        self.call("input", target=control, kind="click")

    def close(self):
        if self.process and self.process.poll() is None:
            self.call("close_app")
            self.process.wait(timeout=15)
        if self.process:
            assert self.process.returncode == 0, "Diagnostic app did not exit normally"
        for stream in self.streams:
            stream.close()
        (self.root / "session.json").unlink(missing_ok=True)

    def cleanup(self):
        failures = []
        for process, stop in self.children:
            try:
                stop.write_text("stop", encoding="utf-8")
                process.wait(timeout=5)
            except Exception:
                failures.append("workload required forced cleanup")
                try:
                    process.kill()  # The retained Popen handle, never a basename/PID search.
                    process.wait(timeout=5)
                except Exception:
                    failures.append("workload cleanup failed")
            finally:
                if hasattr(process, "close"):
                    process.close()
        try:
            self.close()
        except Exception:
            if self.process and self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)
            failures.append("app required forced cleanup")
        finally:
            for stream in self.streams:
                stream.close()
            (self.root / "session.json").unlink(missing_ok=True)
            authority = self.root / "a"
            if authority.exists():
                assert authority.resolve().is_relative_to(self.root)
                shutil.rmtree(authority)
        assert not failures, "; ".join(failures)
