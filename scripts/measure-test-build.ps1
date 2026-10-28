[CmdletBinding()]
param(
    [ValidateRange(1, 5)][int]$Repeats = 2,
    [switch]$Offline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'target'))
$reportRoot = Join-Path $targetRoot ('test-build-timing-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $reportRoot | Out-Null
$report = [ordered]@{
    methodology = 'Fresh root-package test rebuild; dependencies cached equally; no production build timing claim'
    cargo = (& cargo --version)
    rustc = (& rustc --version)
    commit = (& git -C $repoRoot rev-parse HEAD)
    runs = @()
}
Push-Location -LiteralPath $repoRoot
try {
    for ($round = 1; $round -le $Repeats; $round++) {
        # Reverse the order to reduce a one-direction cache/order bias.
        $variants = if ($round % 2 -eq 1) { @('before', 'after') } else { @('after', 'before') }
        foreach ($variant in $variants) {
            $optimization = if ($variant -eq 'before') { 3 } else { 1 }
            $debugInfo = if ($variant -eq 'before') { 2 } else { 1 }
            $name = "$round-$variant"
            # Cargo removes only this package's compiled test/dev artifacts, not dependencies or sources.
            & cargo clean --target-dir $targetRoot --package cpu-affinity-tool --profile test *> (Join-Path $reportRoot "$name-clean.log")
            if ($LASTEXITCODE -ne 0) { throw 'Root-package artifact cleanup failed.' }
            $platform = if ($env:OS -eq 'Windows_NT') { 'windows' } else { 'linux' }
            $binary = if ($platform -eq 'windows') { 'cpu-affinity-tool' } else { 'cpu-affinity-tool-linux' }
            $cargoArguments = @('test', '--locked', '--target-dir', $targetRoot, '--features', $platform, '--bin', $binary,
                '--config', "profile.test.package.cpu-affinity-tool.opt-level=$optimization",
                '--config', "profile.test.package.cpu-affinity-tool.debug=$debugInfo")
            if ($Offline) { $cargoArguments += '--offline' }
            $clock = [Diagnostics.Stopwatch]::StartNew()
            & cargo @cargoArguments --no-run *> (Join-Path $reportRoot "$name-build.log")
            $buildCode = $LASTEXITCODE
            $clock.Stop()
            $buildSeconds = $clock.Elapsed.TotalSeconds
            if ($buildCode -ne 0) { throw "Test compilation failed: $name" }
            $clock.Restart()
            & cargo @cargoArguments *> (Join-Path $reportRoot "$name-test.log")
            $testCode = $LASTEXITCODE
            $clock.Stop()
            $report.runs += [ordered]@{ round = $round; variant = $variant; build_seconds = $buildSeconds;
                test_command_seconds = $clock.Elapsed.TotalSeconds; build_exit = $buildCode; test_exit = $testCode }
            $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $reportRoot 'result.json') -Encoding utf8
            if ($testCode -ne 0) { throw "Tests failed: $name" }
        }
    }
    $report | ConvertTo-Json -Depth 10
    Write-Host "Timing evidence: $reportRoot"
} finally { Pop-Location }
