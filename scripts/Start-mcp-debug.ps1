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

function Write-McpDebugState {
    param([string]$Source, [string]$Destination)
    if ([IO.Path]::GetFullPath($Source) -eq [IO.Path]::GetFullPath($Destination)) {
        throw 'The debug package must be in its own folder, separate from the original state.json.'
    }
    $saved = Get-Content -Raw -LiteralPath $Source | ConvertFrom-Json
    [IO.File]::WriteAllText($Destination, ((New-McpDebugState $saved) | ConvertTo-Json -Depth 100), (New-Object Text.UTF8Encoding($false)))
}

if ($SelfTest) {
    $saved = '{"version":11,"groups":[{"programs":[{"autorun":true},{}]}],"tuning":{"enabled":true},"unknown":42}' | ConvertFrom-Json
    $copy = New-McpDebugState $saved
    if ($copy.Contains('tuning') -or $copy.Contains('unknown') -or $copy.process_monitoring_enabled -or $copy.groups[0].programs[0].autorun -or $copy.groups[0].programs[1].autorun) { throw 'Unsafe debug state' }
    if (-not $saved.groups[0].programs[0].autorun -or -not $copy.cpu_load_monitoring_enabled -or $copy.cpu_load_history_seconds -ne 300) { throw 'Input mutation or invalid defaults' }
    $testRoot = Join-Path ([IO.Path]::GetTempPath()) ('cpu-affinity-mcp-paths-' + [Guid]::NewGuid().ToString('N'))
    $testSource = Join-Path $testRoot 'state.json'
    $testDestination = Join-Path $testRoot 'debug.json'
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    $original = '{"version":11,"groups":[{"programs":[{"autorun":true}]}],"tuning":{"keep":true},"extension":42}'
    try {
        [IO.File]::WriteAllText($testSource, $original)
        foreach ($overlap in @($testSource, (Join-Path $testRoot '.\STATE.JSON'))) {
            $rejected = $false
            try { Write-McpDebugState -Source $testSource -Destination $overlap } catch { $rejected = $true }
            if (-not $rejected -or [IO.File]::ReadAllText($testSource) -cne $original) { throw 'Overlapping debug state must be rejected without changing original bytes' }
        }
        Write-McpDebugState -Source $testSource -Destination $testDestination
        $written = Get-Content -Raw -LiteralPath $testDestination | ConvertFrom-Json
        if ($written.process_monitoring_enabled -or $written.groups[0].programs[0].autorun -or $null -ne $written.PSObject.Properties['tuning']) { throw 'Unsafe written debug state' }
        if ([IO.File]::ReadAllText($testSource) -cne $original) { throw 'Separate debug state changed original bytes' }
    }
    finally {
        foreach ($testFile in @($testSource, $testDestination)) { if (Test-Path -LiteralPath $testFile) { Remove-Item -LiteralPath $testFile } }
        Remove-Item -LiteralPath $testRoot
    }
    Write-Output 'PASS: isolated settings, immutable input, no autorun/enforcement/tuning; overlapping paths rejected before write'
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
    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    $utf8 = New-Object Text.UTF8Encoding($false)
    Write-McpDebugState -Source $source -Destination (Join-Path $debugRoot 'state.json')
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
