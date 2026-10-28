[CmdletBinding()]
param(
    [string]$Python = 'python',
    [string]$ExePath,
    [switch]$InteractiveLaunch,
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'The live MCP scenarios require Windows.' }
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this diagnostic-only suite from an elevated PowerShell: protected MCP authority and ETW require administrator rights.'
}
$repoRoot = Split-Path -Parent $PSScriptRoot
$runRoot = Join-Path $repoRoot ('target/lm-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$compiler = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
Push-Location -LiteralPath $repoRoot
try {
    if (-not $SkipBuild) {
        & cargo build --locked --features windows,debug-mcp,diagnostics-test-controls --bin cpu-affinity-tool
        if ($LASTEXITCODE -ne 0) { throw 'Diagnostic live MCP build failed.' }
    }
    if (-not $ExePath) { $ExePath = Join-Path $repoRoot 'target/debug/cpu-affinity-tool.exe' }
    New-Item -ItemType Directory -Path $runRoot | Out-Null
    $workload = Join-Path $runRoot 'workload.exe'
    & $compiler /nologo /target:exe /optimize+ /reference:System.Windows.Forms.dll "/out:$workload" (Join-Path $PSScriptRoot 'fixtures/live-workload.cs')
    if ($LASTEXITCODE -ne 0) { throw 'Owned workload compilation failed.' }
    $runnerArguments = @((Join-Path $PSScriptRoot 'run_live_mcp.py'), '--exe', $ExePath, '--workload', $workload, '--output', (Join-Path $runRoot 'evidence'))
    if ($InteractiveLaunch) { $runnerArguments += '--interactive-launch' }
    & $Python @runnerArguments
    if ($LASTEXITCODE -ne 0) { throw "Live MCP acceptance failed. Evidence: $runRoot/evidence/result.json" }
    Write-Host "Live MCP acceptance passed. Evidence: $runRoot/evidence/result.json"
} finally {
    Pop-Location
}
