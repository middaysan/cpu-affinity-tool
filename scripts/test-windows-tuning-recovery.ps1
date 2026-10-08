[CmdletBinding()]
param(
    [switch]$Offline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($env:OS -ne "Windows_NT") {
    throw "The native tuning recovery probes require Windows."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$testPrefix = "app::features::tuning::recovery_tests::"
$requiredTests = @(
    "${testPrefix}forced_exit_after_intent_recovers_without_a_target_write",
    "${testPrefix}forced_exit_after_physical_write_restores_exact_original_baseline",
    "${testPrefix}forced_exit_after_verified_progress_reloads_and_restores_original"
)
$cargoArguments = @(
    "test", "--locked", "--features", "windows,tuning-test-controls",
    "--bin", "cpu-affinity-tool"
)
if ($Offline) {
    $cargoArguments += "--offline"
}
$cargoArguments += $testPrefix

Push-Location -LiteralPath $repoRoot
try {
    # A renamed test or a cfg change must fail the gate instead of passing zero tests.
    $listedTests = @(& cargo @cargoArguments "--" "--list")
    if ($LASTEXITCODE -ne 0) {
        throw "Listing the native tuning recovery probes failed."
    }
    foreach ($requiredTest in $requiredTests) {
        if ($listedTests -notcontains "${requiredTest}: test") {
            throw "Missing native tuning recovery probe: $requiredTest"
        }
    }

    # Rust fixtures own their fresh native objects and child handles, including cleanup.
    & cargo @cargoArguments "--" "--ignored" "--test-threads=1" "--nocapture"
    if ($LASTEXITCODE -ne 0) {
        throw "The native tuning recovery probes failed."
    }
    Write-Host "Windows isolated tuning recovery probes passed."
}
finally {
    Pop-Location
}
