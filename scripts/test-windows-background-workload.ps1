[CmdletBinding()]
param(
    [switch]$Offline,
    [switch]$OwnedProcessProbe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($env:OS -ne "Windows_NT") {
    throw "The background-workload probes require Windows."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$testPrefix = "windows::background::native_tests::function_progress::"
$nativeTest = "${testPrefix}native_owned_background_function_progress"
$pureTests = @(
    "${testPrefix}workload::tests::work_progress_rejects_a_heartbeat_without_the_computed_result",
    "${testPrefix}workload::tests::work_progress_refuses_replay_skips_and_wrong_totals_without_advancing",
    "${testPrefix}workload::tests::work_commands_and_reports_have_strict_finite_bounds",
    "${testPrefix}workload::tests::independent_checksum_matches_fixed_mathematical_vectors",
    "${testPrefix}workload::tests::bounded_reader_refuses_partial_and_oversized_lines"
)
$gateTest = "background::tests::identity_verification_does_not_certify_system_role_mutation"
$cargoArguments = @(
    "test", "--locked", "--manifest-path", "libs/os_api/Cargo.toml",
    "--features", "tuning-test-controls"
)
if ($Offline) {
    $cargoArguments += "--offline"
}

function Assert-TestInventory {
    param([string]$Filter, [string[]]$Required)

    $listedTests = @(& cargo @cargoArguments $Filter "--" "--list")
    if ($LASTEXITCODE -ne 0) {
        throw "Listing background-workload tests failed: $Filter"
    }
    foreach ($requiredTest in $Required) {
        if ($listedTests -notcontains "${requiredTest}: test") {
            throw "Missing background-workload test: $requiredTest"
        }
    }
}

Push-Location -LiteralPath $repoRoot
try {
    # A renamed/cfg-hidden test must fail rather than pass a zero-test invocation.
    Assert-TestInventory -Filter $testPrefix -Required $pureTests
    Assert-TestInventory -Filter "background::tests::" -Required @($gateTest)
    & cargo @cargoArguments $testPrefix "--" "--test-threads=1" "--nocapture"
    if ($LASTEXITCODE -ne 0) {
        throw "The bounded work-protocol tests failed."
    }
    & cargo @cargoArguments "background::tests::" "--" "--test-threads=1" "--nocapture"
    if ($LASTEXITCODE -ne 0) {
        throw "The background admission/decision tests failed."
    }

    if ($OwnedProcessProbe) {
        Assert-TestInventory -Filter $nativeTest -Required @($nativeTest)
        # The fixture holds its own child handle, changes only that process and
        # requires computed work plus exact readback/Undo and normal child exit.
        # This switch grants no authority over Windows roles or services.
        & cargo @cargoArguments $nativeTest "--" "--exact" "--ignored" "--test-threads=1" "--nocapture"
        if ($LASTEXITCODE -ne 0) {
            throw "The owned-child functional probe failed; no role admission is established."
        }
        Write-Host "Owned-child functional progress/readback/Undo passed. Real Windows roles remain unqualified."
    }
    else {
        Write-Host "Pure work-protocol/admission tests passed. Native writes were not requested."
    }
}
finally {
    Pop-Location
}
