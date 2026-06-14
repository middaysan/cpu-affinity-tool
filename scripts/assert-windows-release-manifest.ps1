[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Format-Win32Error {
    param([string]$Operation)

    $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $message = [ComponentModel.Win32Exception]::new($code).Message
    return "$Operation failed with Win32 error $code`: $message"
}

function Convert-ManifestBytesToString {
    param([byte[]]$Bytes)

    if ($Bytes.Length -ge 3 -and $Bytes[0] -eq 0xEF -and $Bytes[1] -eq 0xBB -and $Bytes[2] -eq 0xBF) {
        return [Text.Encoding]::UTF8.GetString($Bytes, 3, $Bytes.Length - 3)
    }

    if ($Bytes.Length -ge 2 -and $Bytes[0] -eq 0xFF -and $Bytes[1] -eq 0xFE) {
        return [Text.Encoding]::Unicode.GetString($Bytes, 2, $Bytes.Length - 2)
    }

    if ($Bytes.Length -ge 2 -and $Bytes[0] -eq 0xFE -and $Bytes[1] -eq 0xFF) {
        return [Text.Encoding]::BigEndianUnicode.GetString($Bytes, 2, $Bytes.Length - 2)
    }

    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    return $utf8.GetString($Bytes)
}

$resolved = Resolve-Path -LiteralPath $Path
$exePath = $resolved.ProviderPath

if (-not ("CpuAffinityTool.NativeResource" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

namespace CpuAffinityTool
{
    public static class NativeResource
    {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr LoadLibraryEx(string lpFileName, IntPtr hFile, uint dwFlags);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern IntPtr FindResource(IntPtr hModule, IntPtr lpName, IntPtr lpType);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern IntPtr LoadResource(IntPtr hModule, IntPtr hResInfo);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern IntPtr LockResource(IntPtr hResData);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern uint SizeofResource(IntPtr hModule, IntPtr hResInfo);

        [DllImport("kernel32.dll", SetLastError = true)]
        public static extern bool FreeLibrary(IntPtr hModule);
    }
}
"@
}

$loadLibraryAsDataFile = 0x00000002
$rtManifest = [IntPtr]24
$createProcessManifestResourceId = [IntPtr]1

$module = [CpuAffinityTool.NativeResource]::LoadLibraryEx($exePath, [IntPtr]::Zero, $loadLibraryAsDataFile)
if ($module -eq [IntPtr]::Zero) {
    throw (Format-Win32Error "LoadLibraryEx($exePath)")
}

try {
    $resourceInfo = [CpuAffinityTool.NativeResource]::FindResource(
        $module,
        $createProcessManifestResourceId,
        $rtManifest
    )
    if ($resourceInfo -eq [IntPtr]::Zero) {
        throw (Format-Win32Error "FindResource(RT_MANIFEST #1)")
    }

    $size = [CpuAffinityTool.NativeResource]::SizeofResource($module, $resourceInfo)
    if ($size -eq 0) {
        throw (Format-Win32Error "SizeofResource(RT_MANIFEST #1)")
    }
    if ($size -gt [int]::MaxValue) {
        throw "RT_MANIFEST resource is too large to inspect safely: $size bytes"
    }

    $resourceHandle = [CpuAffinityTool.NativeResource]::LoadResource($module, $resourceInfo)
    if ($resourceHandle -eq [IntPtr]::Zero) {
        throw (Format-Win32Error "LoadResource(RT_MANIFEST #1)")
    }

    $resourcePointer = [CpuAffinityTool.NativeResource]::LockResource($resourceHandle)
    if ($resourcePointer -eq [IntPtr]::Zero) {
        throw "LockResource(RT_MANIFEST #1) returned null"
    }

    $manifestBytes = [byte[]]::new([int]$size)
    [Runtime.InteropServices.Marshal]::Copy($resourcePointer, $manifestBytes, 0, [int]$size)
    $manifestText = Convert-ManifestBytesToString $manifestBytes

    [xml]$manifestXml = $manifestText
    $requestedExecutionLevel = $manifestXml.SelectSingleNode("//*[local-name()='requestedExecutionLevel']")
    if ($null -eq $requestedExecutionLevel) {
        throw "RT_MANIFEST does not contain requestedExecutionLevel"
    }

    $level = $requestedExecutionLevel.GetAttribute("level")
    $uiAccess = $requestedExecutionLevel.GetAttribute("uiAccess")

    if ($level -ne "requireAdministrator") {
        throw "Expected requestedExecutionLevel level='requireAdministrator', found '$level'"
    }

    if ($uiAccess -ne "false") {
        throw "Expected requestedExecutionLevel uiAccess='false', found '$uiAccess'"
    }

    Write-Host "Verified RT_MANIFEST in $exePath`: requireAdministrator, uiAccess=false"
}
finally {
    [void][CpuAffinityTool.NativeResource]::FreeLibrary($module)
}
