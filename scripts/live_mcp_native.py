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


class DeniedScheduling:
    """Temporarily deny SET_INFORMATION on one retained Popen child; restore its DACL."""
    def __init__(self, child):
        assert isinstance(child, subprocess.Popen) and child.poll() is None
        self.handle = wintypes.HANDLE(int(child._handle))  # Exact CreateProcess handle, not a PID open.
        self.api = ctypes.WinDLL('advapi32', use_last_error=True)
        self.kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        self.kernel.LocalFree.argtypes = [ctypes.c_void_p]
        self.api.GetKernelObjectSecurity.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p,
                                                     wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
        self.api.SetKernelObjectSecurity.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p]
        self.api.ConvertSecurityDescriptorToStringSecurityDescriptorW.argtypes = [ctypes.c_void_p,
            wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.DWORD)]
        self.api.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [wintypes.LPCWSTR,
            wintypes.DWORD, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.DWORD)]
        size = wintypes.DWORD()
        self.api.GetKernelObjectSecurity(self.handle, 4, None, 0, ctypes.byref(size))
        assert 20 <= size.value <= 65536, 'Cannot bound owned process DACL'
        self.original = ctypes.create_string_buffer(size.value)
        assert self.api.GetKernelObjectSecurity(self.handle, 4, self.original, size, ctypes.byref(size))
        self.original_text = self.text(self.original)
        assert self.original_text.startswith('D:') and '(' in self.original_text
        pos = self.original_text.index('(')
        text = self.original_text[:pos] + '(D;;0x0200;;;WD)' + self.original_text[pos:]
        denied = ctypes.c_void_p()
        assert self.api.ConvertStringSecurityDescriptorToSecurityDescriptorW(text, 1, ctypes.byref(denied), None)
        self.active = False
        try:
            assert self.api.SetKernelObjectSecurity(self.handle, 4, denied), 'Cannot deny owned scheduling'
            self.active = True
        finally:
            self.kernel.LocalFree(denied)

    def text(self, descriptor):
        pointer = ctypes.c_void_p()
        assert self.api.ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor, 1, 4,
                                                                            ctypes.byref(pointer), None)
        try:
            return ctypes.wstring_at(pointer)
        finally:
            self.kernel.LocalFree(pointer)

    def restore(self):
        if self.active:
            assert self.api.SetKernelObjectSecurity(self.handle, 4, self.original), 'Owned DACL restore failed'
            self.active = False
            size = wintypes.DWORD(len(self.original))
            current = ctypes.create_string_buffer(size.value)
            assert self.api.GetKernelObjectSecurity(self.handle, 4, current, size, ctypes.byref(size))
            assert self.text(current) == self.original_text, 'Owned DACL readback changed'


class ScopedDebugPrivilege:
    """Disable debug bypass only on this runner/its retained diagnostic child; restore it."""
    class Luid(ctypes.Structure):
        _fields_ = [('low', wintypes.DWORD), ('high', wintypes.LONG)]

    def __init__(self, child=None):
        self.kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        self.kernel.GetCurrentProcess.restype = wintypes.HANDLE
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.api = ctypes.WinDLL('advapi32', use_last_error=True)
        self.api.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE)]
        self.api.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                                wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)]
        self.api.LookupPrivilegeValueW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR, ctypes.POINTER(self.Luid)]
        self.api.AdjustTokenPrivileges.argtypes = [wintypes.HANDLE, wintypes.BOOL, ctypes.c_void_p,
                                                  wintypes.DWORD, ctypes.c_void_p, ctypes.c_void_p]
        assert child is None or isinstance(child, subprocess.Popen) and child.poll() is None
        handle = self.kernel.GetCurrentProcess() if child is None else wintypes.HANDLE(int(child._handle))
        self.token = wintypes.HANDLE()
        assert self.api.OpenProcessToken(handle, 0x20 | 0x8, ctypes.byref(self.token))
        self.changed = False
        try:
            luid = self.Luid()
            assert self.api.LookupPrivilegeValueW(None, 'SeDebugPrivilege', ctypes.byref(luid))
            self.luid = luid
            size = wintypes.DWORD()
            self.api.GetTokenInformation(self.token, 3, None, 0, ctypes.byref(size))
            assert 4 <= size.value <= 65536
            data = ctypes.create_string_buffer(size.value)
            assert self.api.GetTokenInformation(self.token, 3, data, size, ctypes.byref(size))
            count, = struct.unpack_from('<I', data.raw)
            assert 4 + count * 12 <= size.value
            self.previous = next((flags for low, high, flags in
                                  (struct.unpack_from('<IiI', data.raw, 4 + i * 12) for i in range(count))
                                  if (low, high) == (luid.low, luid.high)), None)
            self.was_enabled = self.previous is not None and bool(self.previous & 2)
            if self.was_enabled:
                self.set_flags(self.previous & ~2)
                self.changed = True
        except BaseException:
            self.kernel.CloseHandle(self.token)
            raise

    def set_flags(self, flags):
        state = ctypes.create_string_buffer(struct.pack('<IIiI', 1, self.luid.low, self.luid.high, flags))
        ctypes.set_last_error(0)
        assert self.api.AdjustTokenPrivileges(self.token, False, state, 0, None, None)
        assert ctypes.get_last_error() == 0, 'Owned debug privilege update failed'

    def restore(self):
        try:
            if self.changed:
                self.set_flags(self.previous)
                self.changed = False
        finally:
            self.kernel.CloseHandle(self.token)
