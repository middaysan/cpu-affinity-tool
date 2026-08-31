[CmdletBinding()]
param(
    [string]$TargetDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$debugVariable = "CARGO_PROFILE_RELEASE_DEBUG"
$targetVariable = "CARGO_TARGET_DIR"
$previousDebug = [Environment]::GetEnvironmentVariable($debugVariable, "Process")
$previousTarget = [Environment]::GetEnvironmentVariable($targetVariable, "Process")

try {
    [Environment]::SetEnvironmentVariable(
        $debugVariable,
        "line-tables-only",
        "Process"
    )

    if (-not [string]::IsNullOrWhiteSpace($TargetDir)) {
        $resolvedTargetDir = [IO.Path]::GetFullPath($TargetDir)
        [Environment]::SetEnvironmentVariable($targetVariable, $resolvedTargetDir, "Process")
    }

    & cargo build --release --features windows --bin cpu-affinity-tool
    if ($LASTEXITCODE -ne 0) {
        throw "Windows release build failed with exit code $LASTEXITCODE"
    }
}
finally {
    [Environment]::SetEnvironmentVariable($debugVariable, $previousDebug, "Process")
    [Environment]::SetEnvironmentVariable($targetVariable, $previousTarget, "Process")
}
