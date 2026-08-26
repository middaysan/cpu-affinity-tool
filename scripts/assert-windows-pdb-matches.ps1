[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [Parameter(Mandatory = $true)]
    [string]$PdbPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RequiredNonEmptyFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CandidatePath,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    if (-not (Test-Path -LiteralPath $CandidatePath -PathType Leaf)) {
        throw "Missing $Description`: $CandidatePath"
    }

    $resolvedPath = (Resolve-Path -LiteralPath $CandidatePath).ProviderPath
    if ((Get-Item -LiteralPath $resolvedPath).Length -eq 0) {
        throw "$Description is empty: $resolvedPath"
    }

    return $resolvedPath
}

if (-not ("CpuAffinityTool.Symbols.DbgHelp" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

namespace CpuAffinityTool.Symbols
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct SymSrvIndexInfo
    {
        public UInt32 sizeofstruct;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 261)]
        public string file;

        [MarshalAs(UnmanagedType.Bool)]
        public bool stripped;

        public UInt32 timestamp;
        public UInt32 size;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 261)]
        public string dbgfile;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 261)]
        public string pdbfile;

        public Guid guid;
        public UInt32 sig;
        public UInt32 age;
    }

    public static class DbgHelp
    {
        [DllImport(
            "dbghelp.dll",
            EntryPoint = "SymSrvGetFileIndexInfoW",
            CharSet = CharSet.Unicode,
            SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SymSrvGetFileIndexInfo(
            string file,
            ref SymSrvIndexInfo info,
            UInt32 flags);

        public static SymSrvIndexInfo ReadIndex(string path)
        {
            var info = new SymSrvIndexInfo
            {
                sizeofstruct = (UInt32)Marshal.SizeOf(typeof(SymSrvIndexInfo))
            };

            if (!SymSrvGetFileIndexInfo(path, ref info, 0))
            {
                throw new Win32Exception(
                    Marshal.GetLastWin32Error(),
                    "SymSrvGetFileIndexInfoW failed for " + path);
            }

            return info;
        }
    }
}
"@
}

$resolvedExePath = Resolve-RequiredNonEmptyFile -CandidatePath $ExePath -Description "Windows executable"
$resolvedPdbPath = Resolve-RequiredNonEmptyFile -CandidatePath $PdbPath -Description "Windows PDB"

$exeIndex = [CpuAffinityTool.Symbols.DbgHelp]::ReadIndex($resolvedExePath)
$pdbIndex = [CpuAffinityTool.Symbols.DbgHelp]::ReadIndex($resolvedPdbPath)
$embeddedPdbName = [IO.Path]::GetFileName($exeIndex.pdbfile)
$actualPdbName = [IO.Path]::GetFileName($resolvedPdbPath)

if ([string]::IsNullOrWhiteSpace($embeddedPdbName)) {
    throw "Windows executable does not name a PDB in its CodeView record: $resolvedExePath"
}
if (-not [StringComparer]::OrdinalIgnoreCase.Equals($embeddedPdbName, $actualPdbName)) {
    throw "Windows PDB basename mismatch: executable expects '$embeddedPdbName', received '$actualPdbName'"
}
if ($exeIndex.guid -eq [Guid]::Empty -or $pdbIndex.guid -eq [Guid]::Empty) {
    throw "Windows EXE/PDB identity is missing a CodeView GUID"
}
if ($exeIndex.guid -ne $pdbIndex.guid -or $exeIndex.age -ne $pdbIndex.age) {
    throw (
        "Windows EXE/PDB identity mismatch: " +
        "EXE GUID=$($exeIndex.guid), age=$($exeIndex.age); " +
        "PDB GUID=$($pdbIndex.guid), age=$($pdbIndex.age)"
    )
}

Write-Host (
    "Verified Windows EXE/PDB identity: " +
    "$actualPdbName, GUID=$($exeIndex.guid), age=$($exeIndex.age)"
)
