"""Retained exact handles for test-owned managed children, never process-name cleanup."""
import ctypes
import socket
import struct
import subprocess
from ctypes import wintypes


def available_cpus():
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.GetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)]
    process, system = ctypes.c_size_t(), ctypes.c_size_t()
    assert kernel.GetProcessAffinityMask(kernel.GetCurrentProcess(), ctypes.byref(process), ctypes.byref(system))
    return [cpu for cpu in range(ctypes.sizeof(ctypes.c_size_t) * 8) if process.value & (1 << cpu)]


def pin_workload(process, cpu):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.SetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.c_size_t]
    kernel.SetProcessAffinityMask.restype = wintypes.BOOL
    assert kernel.SetProcessAffinityMask(wintypes.HANDLE(int(process._handle)), 1 << cpu), "Cannot place the owned workload"


def listener_pid(port):
    api = ctypes.WinDLL("iphlpapi", use_last_error=True).GetExtendedTcpTable
    api.argtypes = [ctypes.c_void_p, ctypes.POINTER(wintypes.DWORD), wintypes.BOOL,
                    wintypes.ULONG, ctypes.c_int, wintypes.ULONG]
    api.restype = wintypes.DWORD
    size = wintypes.DWORD()
    result = api(None, ctypes.byref(size), False, 2, 3, 0)  # IPv4 OWNER_PID_LISTENER.
    if result not in (0, 122) or not 4 <= size.value <= 4 * 1024 * 1024:
        raise AssertionError("Cannot bound listener ownership readback")
    buffer = ctypes.create_string_buffer(size.value)
    result = api(buffer, ctypes.byref(size), False, 2, 3, 0)
    if result == 122:
        return None  # Table changed; the caller's bounded readiness wait retries this read only.
    assert result == 0, "Listener ownership readback failed"
    count = struct.unpack_from("<I", buffer)[0]
    assert 4 + count * 24 <= len(buffer), "Invalid listener table bounds"
    owners = set()
    for i in range(count):
        _, address, local_port, _, _, pid = struct.unpack_from("<6I", buffer, 4 + i * 24)
        if socket.ntohs(local_port & 0xffff) == port and address in (0, 0x0100007f):
            owners.add(pid)
    assert len(owners) <= 1, "Ambiguous listener ownership"
    return next(iter(owners), None)


class OwnedProcess:
    def __init__(self, pid, creation_token):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.kernel.OpenProcess.restype = wintypes.HANDLE
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        self.kernel.GetProcessTimes.restype = wintypes.BOOL
        self.kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        self.kernel.WaitForSingleObject.restype = wintypes.DWORD
        self.kernel.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self.kernel.TerminateProcess.restype = wintypes.BOOL
        self.kernel.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        self.kernel.GetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)]
        self.kernel.GetProcessAffinityMask.restype = wintypes.BOOL
        self.kernel.GetPriorityClass.argtypes = [wintypes.HANDLE]
        self.kernel.GetPriorityClass.restype = wintypes.DWORD
        self.pid = pid
        self.handle = self.kernel.OpenProcess(0x100000 | 0x1000 | 1, False, pid)
        if not self.handle:
            raise AssertionError("Cannot retain the owned managed child")
        times = [wintypes.FILETIME() for _ in range(4)]
        if not self.kernel.GetProcessTimes(self.handle, *(ctypes.byref(t) for t in times)):
            self.close()
            raise AssertionError("Cannot verify the managed child lifetime")
        actual = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
        if actual != int(creation_token):
            self.close()
            raise AssertionError("Managed child lifetime changed")

    def scheduling(self):
        process, system = ctypes.c_size_t(), ctypes.c_size_t()
        assert self.kernel.GetProcessAffinityMask(self.handle, ctypes.byref(process), ctypes.byref(system))
        return process.value, self.kernel.GetPriorityClass(self.handle)

    def wait(self, timeout):
        if self.kernel.WaitForSingleObject(self.handle, round(timeout * 1000)) == 258:
            raise subprocess.TimeoutExpired("owned native child", timeout)
        code = wintypes.DWORD()
        assert self.kernel.GetExitCodeProcess(self.handle, ctypes.byref(code))
        return code.value

    def kill(self):
        assert self.kernel.TerminateProcess(self.handle, 125)

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None
