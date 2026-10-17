[CmdletBinding()]
param([ValidateRange(10,900)][int]$Seconds = 300, [switch]$SelfTest)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$profileRoot = $PSScriptRoot
$profileExe = Join-Path $profileRoot 'cpu-affinity-tool.exe'

function New-RenderTestState {
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
    $saved = '{"version":11,"groups":[{"name":"test","programs":[{"autorun":true},{"name":"legacy"}]}],"cpu_schema":{},"cpu_load_history_seconds":1800,"tuning":{"secret":"excluded"},"unknown":"excluded"}' | ConvertFrom-Json
    $result = New-RenderTestState $saved
    if ($result.Contains('tuning') -or $result.Contains('unknown')) { throw 'Unsafe fields retained' }
    if ($result.groups[0].programs[0].autorun -or $result.groups[0].programs[1].autorun) { throw 'Autorun retained' }
    if (-not $saved.groups[0].programs[0].autorun) { throw 'Input mutated' }
    if ($result.process_monitoring_enabled -or $result.start_minimized -or $result.windows_event_log_diagnostics_enabled) { throw 'Background actions retained' }
    if (-not $result.cpu_load_monitoring_enabled -or $result.cpu_load_history_seconds -ne 1800) { throw 'History preference lost' }
    $saved.PSObject.Properties.Remove('cpu_load_history_seconds')
    if ((New-RenderTestState $saved).cpu_load_history_seconds -ne 300) { throw 'Missing duration fallback failed' }
    Write-Output 'PASS: isolated state, absent legacy autorun, safe fields, defaults and immutable input'
    exit 0
}

try {
    if (-not (Test-Path -LiteralPath $profileExe -PathType Leaf)) {
        throw 'Keep this script beside the diagnostic cpu-affinity-tool.exe.'
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        $launchArgs = @('-NoProfile','-ExecutionPolicy','Bypass','-File',('"' + $PSCommandPath + '"'),'-Seconds',$Seconds)
        Start-Process -FilePath 'powershell.exe' -Verb RunAs -WindowStyle Hidden -ArgumentList $launchArgs | Out-Null
        exit 0
    }

    $running = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
        $_.MainWindowTitle -eq 'CPU Affinity Tool' -or $_.ProcessName -eq 'cpu-affinity-tool'
    })
    if ($running.Count -gt 0) { throw 'Close the currently running CPU Affinity Tool, including its tray instance, then run this script again.' }

    $desktopState = Join-Path ([Environment]::GetFolderPath('Desktop')) 'state.json'
    $platformState = Join-Path $env:LOCALAPPDATA 'CpuAffinityTool\state.json'
    $sourceState = if (Test-Path -LiteralPath $desktopState) { $desktopState } else { $platformState }
    if (-not (Test-Path -LiteralPath $sourceState)) { throw 'Saved app settings were not found. Start the ordinary app once before this test.' }
    if ([IO.Path]::GetFullPath($sourceState) -eq [IO.Path]::GetFullPath((Join-Path $profileRoot 'state.json'))) {
        throw 'The test package must be in its own folder.'
    }
    $originalHash = (Get-FileHash -LiteralPath $sourceState -Algorithm SHA256).Hash
    $saved = Get-Content -Raw -LiteralPath $sourceState | ConvertFrom-Json
    $isolated = New-RenderTestState $saved
    $utf8 = New-Object Text.UTF8Encoding($false)
    [IO.File]::WriteAllText((Join-Path $profileRoot 'state.json'),($isolated | ConvertTo-Json -Depth 100),$utf8)

    $logs = Join-Path $profileRoot 'logs'
    New-Item -ItemType Directory -Force -Path $logs | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
    $log = Join-Path $logs "render-$stamp.jsonl"
    $stdout = Join-Path $logs "startup-$stamp.out.txt"
    $stderr = Join-Path $logs "startup-$stamp.err.txt"
    $previousLog = [Environment]::GetEnvironmentVariable('CPU_AFFINITY_RENDER_LOG','Process')
    $previousSeconds = [Environment]::GetEnvironmentVariable('CPU_AFFINITY_RENDER_SECONDS','Process')
    try {
        [Environment]::SetEnvironmentVariable('CPU_AFFINITY_RENDER_LOG',$log,'Process')
        [Environment]::SetEnvironmentVariable('CPU_AFFINITY_RENDER_SECONDS',"$Seconds",'Process')
        $app = Start-Process -FilePath $profileExe -WorkingDirectory $profileRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        $app.WaitForExit()
        $exitCode = $app.ExitCode
    }
    finally {
        [Environment]::SetEnvironmentVariable('CPU_AFFINITY_RENDER_LOG',$previousLog,'Process')
        [Environment]::SetEnvironmentVariable('CPU_AFFINITY_RENDER_SECONDS',$previousSeconds,'Process')
    }
    if ((Get-FileHash -LiteralPath $sourceState -Algorithm SHA256).Hash -ne $originalHash) {
        throw 'The original settings changed while the test was running. They were not restored or overwritten.'
    }
    if (-not (Test-Path -LiteralPath $log)) { throw "The profiling log was not created. Read $stderr" }
    $lastLine = Get-Content -LiteralPath $log -Tail 1 | ConvertFrom-Json
    if ($lastLine.kind -ne 'session_end') { throw "The log is incomplete; keep it for analysis. Read $stderr" }
    $summary = [ordered]@{
        exit_code=$exitCode; reason=$lastLine.reason; dropped_records=$lastLine.dropped_records
        diagnostic_exe_sha256=(Get-FileHash -LiteralPath $profileExe -Algorithm SHA256).Hash
        original_settings_unchanged=$true
    }
    $summaryPath = Join-Path $logs "result-$stamp.json"
    [IO.File]::WriteAllText($summaryPath,($summary | ConvertTo-Json),$utf8)
    $zip = Join-Path $profileRoot "render-log-$stamp.zip"
    Compress-Archive -LiteralPath @($log,$summaryPath,$stdout,$stderr) -DestinationPath $zip
    [IO.File]::WriteAllText((Join-Path $profileRoot 'LAST-RESULT.txt'),"Log archive: $zip`r`nExit code: $exitCode`r`nReason: $($lastLine.reason)`r`nDropped records: $($lastLine.dropped_records)`r`n",$utf8)
}
catch {
    [IO.File]::WriteAllText((Join-Path $profileRoot 'TEST-ERROR.txt'),$_.ToString())
    exit 1
}
