[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$binaryPath = Join-Path $repoRoot "target\debug\cpu-affinity-tool.exe"
$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "cpu-affinity-tool-crash-probe-{0}-{1}" -f $PID, [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
)

function Invoke-CrashProbe {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Mode,
        [Parameter(Mandatory = $true)]
        [string]$ExpectedKind,
        [Parameter(Mandatory = $true)]
        [string]$ExpectedPhase,
        [Parameter(Mandatory = $true)]
        [int]$ExpectedExitCode
    )

    $reportDirectory = Join-Path $testRoot $Mode
    [IO.Directory]::CreateDirectory($reportDirectory) | Out-Null

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $binaryPath
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.Environment["CPU_AFFINITY_TOOL_TEST_CRASH_REPORT_DIR"] = $reportDirectory
    $startInfo.Environment["CPU_AFFINITY_TOOL_TEST_CRASH_FAULT"] = $Mode

    $process = [Diagnostics.Process]::Start($startInfo)
    if (-not $process.WaitForExit(10000)) {
        $process.Kill($true)
        throw "Crash probe '$Mode' exceeded the 10 second deadline."
    }
    if ($process.ExitCode -ne $ExpectedExitCode) {
        throw "Crash probe '$Mode' exited with $($process.ExitCode), expected $ExpectedExitCode."
    }

    $reports = @(Get-ChildItem -LiteralPath $reportDirectory -Filter "crash-*.txt" -File)
    if ($reports.Count -ne 1) {
        throw "Crash probe '$Mode' produced $($reports.Count) complete reports, expected 1."
    }
    $content = [IO.File]::ReadAllText($reports[0].FullName)
    if (-not $content.Contains("event_kind: $ExpectedKind")) {
        throw "Crash probe '$Mode' report has the wrong event kind."
    }
    if (-not $content.Contains("startup_phase: $ExpectedPhase")) {
        throw "Crash probe '$Mode' report has the wrong startup phase."
    }
    if (-not $content.EndsWith("--- END CRASH REPORT ---`n")) {
        throw "Crash probe '$Mode' report is missing the completion marker."
    }
    $partialReports = @(Get-ChildItem -LiteralPath $reportDirectory -Filter "*.partial" -File)
    if ($partialReports.Count -ne 0) {
        throw "Crash probe '$Mode' left $($partialReports.Count) partial reports."
    }
}

try {
    & cargo build --features "windows,diagnostics-test-controls" --bin cpu-affinity-tool
    if ($LASTEXITCODE -ne 0) {
        throw "The diagnostics probe build failed."
    }

    [IO.Directory]::CreateDirectory($testRoot) | Out-Null
    Invoke-CrashProbe `
        -Mode "pre-runtime-panic" `
        -ExpectedKind "main_thread_panic" `
        -ExpectedPhase "preparing_runtime" `
        -ExpectedExitCode 101
    Invoke-CrashProbe `
        -Mode "native-loop-error" `
        -ExpectedKind "native_loop_error" `
        -ExpectedPhase "running_ui" `
        -ExpectedExitCode 1

    Write-Host "Windows crash report probes passed."
}
finally {
    $resolvedTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $resolvedTestRoot = [IO.Path]::GetFullPath($testRoot)
    if ($resolvedTestRoot.StartsWith($resolvedTemp, [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Directory]::Exists($resolvedTestRoot)) {
        [IO.Directory]::Delete($resolvedTestRoot, $true)
    }
}
