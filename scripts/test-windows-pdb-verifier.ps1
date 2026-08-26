[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [Parameter(Mandatory = $true)]
    [string]$PdbPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$verifierPath = Join-Path $PSScriptRoot "assert-windows-pdb-matches.ps1"
if (-not (Test-Path -LiteralPath $verifierPath -PathType Leaf)) {
    throw "Missing Windows PDB verifier: $verifierPath"
}

function Assert-ThrowsLike {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Action,

        [Parameter(Mandatory = $true)]
        [string]$Pattern
    )

    try {
        & $Action
    }
    catch {
        if ($_.Exception.Message -notlike $Pattern) {
            throw "Expected failure matching '$Pattern', received: $($_.Exception.Message)"
        }
        return
    }

    throw "Expected action to fail with a message matching '$Pattern'"
}

$resolvedExePath = (Resolve-Path -LiteralPath $ExePath).ProviderPath
$resolvedPdbPath = (Resolve-Path -LiteralPath $PdbPath).ProviderPath

& $verifierPath -ExePath $resolvedExePath -PdbPath $resolvedPdbPath

$testDirectory = Join-Path (
    [IO.Path]::GetTempPath()
) ("cpu-affinity-tool-pdb-verifier-" + [Guid]::NewGuid().ToString("N"))
[void](New-Item -ItemType Directory -Path $testDirectory)

try {
    $missingPdbPath = Join-Path $testDirectory "missing.pdb"
    Assert-ThrowsLike -Pattern "*Missing Windows PDB*" -Action {
        & $verifierPath -ExePath $resolvedExePath -PdbPath $missingPdbPath
    }

    $emptyPdbPath = Join-Path $testDirectory "empty.pdb"
    [void](New-Item -ItemType File -Path $emptyPdbPath)
    Assert-ThrowsLike -Pattern "*Windows PDB is empty*" -Action {
        & $verifierPath -ExePath $resolvedExePath -PdbPath $emptyPdbPath
    }

    $wrongNamePath = Join-Path $testDirectory "wrong-name.pdb"
    Copy-Item -LiteralPath $resolvedPdbPath -Destination $wrongNamePath
    Assert-ThrowsLike -Pattern "*basename mismatch*" -Action {
        & $verifierPath -ExePath $resolvedExePath -PdbPath $wrongNamePath
    }

    $releaseDirectory = Split-Path -Parent $resolvedExePath
    $buildDirectory = Join-Path $releaseDirectory "build"
    $projectBuildDirectories = Get-ChildItem -LiteralPath $buildDirectory -Directory |
        Where-Object { $_.Name -like "cpu-affinity-tool-*" }
    $foreignPdb = $projectBuildDirectories |
        ForEach-Object {
            Get-ChildItem -LiteralPath $_.FullName -File -Filter "build_script_build*.pdb"
        } |
        Select-Object -First 1
    if ($null -eq $foreignPdb) {
        throw "Unable to find the cpu-affinity-tool build-script PDB under $buildDirectory"
    }

    $identityTestDirectory = Join-Path $testDirectory "identity-mismatch"
    [void](New-Item -ItemType Directory -Path $identityTestDirectory)
    $foreignPdbCopy = Join-Path $identityTestDirectory ([IO.Path]::GetFileName($resolvedPdbPath))
    Copy-Item -LiteralPath $foreignPdb.FullName -Destination $foreignPdbCopy
    Assert-ThrowsLike -Pattern "*identity mismatch*" -Action {
        & $verifierPath -ExePath $resolvedExePath -PdbPath $foreignPdbCopy
    }
}
finally {
    $resolvedTestDirectory = [IO.Path]::GetFullPath($testDirectory)
    $resolvedTempDirectory = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $testParentDirectory = [IO.Path]::GetDirectoryName($resolvedTestDirectory)
    $expectedParentDirectory = $resolvedTempDirectory.TrimEnd(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    )
    if (-not [StringComparer]::OrdinalIgnoreCase.Equals(
        $testParentDirectory,
        $expectedParentDirectory
    )) {
        throw "Refusing to remove test directory outside the system temp directory: $resolvedTestDirectory"
    }
    [IO.Directory]::Delete($resolvedTestDirectory, $true)
}

Write-Host "Windows PDB verifier contract tests passed"
