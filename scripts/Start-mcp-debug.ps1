[CmdletBinding()]
param([switch]$SelfTest)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function New-McpDebugState {
    param([Parameter(Mandatory=$true)]$Saved)
    $copy = $Saved | ConvertTo-Json -Depth 100 | ConvertFrom-Json
    $isolated = [ordered]@{}
    foreach ($field in @('version','groups','cpu_schema','theme_index','rule_identities')) {
        if ($null -ne $copy.PSObject.Properties[$field]) { $isolated[$field] = $copy.$field }
    }
    foreach ($group in $isolated.groups) {
        foreach ($program in $group.programs) {
            $program | Add-Member -NotePropertyName autorun -NotePropertyValue $false -Force
        }
    }
    $isolated.process_monitoring_enabled = $false
    $isolated.cpu_load_monitoring_enabled = $true
    $isolated.cpu_load_history_seconds = if ($null -ne $copy.PSObject.Properties['cpu_load_history_seconds']) { $copy.cpu_load_history_seconds } else { 300 }
    $isolated.start_minimized = $false
    $isolated.windows_event_log_diagnostics_enabled = $false
    return $isolated
}

if ($SelfTest) {
    $saved = '{"version":11,"groups":[{"programs":[{"autorun":true},{}]}],"tuning":{"enabled":true},"unknown":42}' | ConvertFrom-Json
    $copy = New-McpDebugState $saved
    if ($copy.Contains('tuning') -or $copy.Contains('unknown') -or $copy.process_monitoring_enabled -or $copy.groups[0].programs[0].autorun -or $copy.groups[0].programs[1].autorun) { throw 'Unsafe debug state' }
    if (-not $saved.groups[0].programs[0].autorun -or -not $copy.cpu_load_monitoring_enabled -or $copy.cpu_load_history_seconds -ne 300) { throw 'Input mutation or invalid defaults' }
    Write-Output 'PASS: isolated settings, immutable input, no autorun/enforcement/tuning'
    exit 0
}

try {
    $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Start-Process -FilePath 'powershell.exe' -Verb RunAs -WindowStyle Hidden -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File',('"' + $PSCommandPath + '"')) | Out-Null
        exit 0
    }
    $debugRoot = $PSScriptRoot
    $exe = Join-Path $debugRoot 'cpu-affinity-tool.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Keep this launcher beside its debug executable.' }
    $session = Join-Path $debugRoot 'mcp-session.json'
    if (Test-Path -LiteralPath $session) {
        $old = Get-Content -Raw -LiteralPath $session | ConvertFrom-Json
        $oldProcess = Get-Process -Id $old.pid -ErrorAction SilentlyContinue
        if ($null -ne $oldProcess -and $oldProcess.Path -eq $exe) { throw 'The previous debug app is still running.' }
        Remove-Item -LiteralPath $session
    }
    $desktop = [Environment]::GetFolderPath('Desktop')
    $sidecar = Join-Path $desktop 'state.json'
    $source = if (Test-Path -LiteralPath $sidecar) { $sidecar } else { Join-Path $env:LOCALAPPDATA 'CpuAffinityTool\state.json' }
    $saved = Get-Content -Raw -LiteralPath $source | ConvertFrom-Json
    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    $utf8 = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText((Join-Path $debugRoot 'state.json'),((New-McpDebugState $saved) | ConvertTo-Json -Depth 100),$utf8)
    [IO.File]::WriteAllText((Join-Path $debugRoot '.cpu-affinity-debug-isolated'),'debug-only',$utf8)
    $logs = Join-Path $debugRoot 'logs'
    New-Item -ItemType Directory -Force -Path $logs | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $random = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($random) } finally { $rng.Dispose() }
    $env:CPU_AFFINITY_MCP_SESSION = $session
    $env:CPU_AFFINITY_MCP_TOKEN = [BitConverter]::ToString($random).Replace('-','').ToLowerInvariant()
    $env:CPU_AFFINITY_RENDER_LOG = Join-Path $logs "render-$stamp.jsonl"
    $env:CPU_AFFINITY_RENDER_SECONDS = '900'
    $app = Start-Process -FilePath $exe -WorkingDirectory $debugRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $logs "startup-$stamp.out.txt") -RedirectStandardError (Join-Path $logs "startup-$stamp.err.txt")
    $null = $app.Handle
    Remove-Item Env:CPU_AFFINITY_MCP_TOKEN
    $app.WaitForExit()
    $app.Refresh()
    [IO.File]::WriteAllText((Join-Path $logs "exit-$stamp.json"),(@{exit_code=$app.ExitCode;original_settings_unchanged=((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -eq $sourceHash)} | ConvertTo-Json),$utf8)
}
catch {
    [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'TEST-ERROR.txt'),$_.ToString())
    exit 1
}
