[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$TestBinary,
    [switch]$PrepareOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'target'))
$resolvedTest = [IO.Path]::GetFullPath($TestBinary)
if (-not $resolvedTest.StartsWith($targetRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or
    -not [IO.Path]::GetFileName($resolvedTest).StartsWith('cpu_affinity_tool-') -or
    -not (Test-Path -LiteralPath $resolvedTest -PathType Leaf)) {
    throw 'Use the application test executable built in this repository target directory.'
}
$fixtureRoot = [IO.Path]::GetFullPath((Join-Path $targetRoot ('known-app-native-' + [guid]::NewGuid().ToString('N'))))
if (-not $fixtureRoot.StartsWith($targetRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Fixture directory escaped target.' }
$steamRoot = Join-Path $fixtureRoot 'Steam'
$discordRoot = Join-Path $fixtureRoot 'Discord'
$helperRoot = Join-Path $steamRoot 'bin\cef\cef.win64'
$foreignRoot = Join-Path $fixtureRoot 'Foreign\bin\cef\cef.win64'
foreach ($directory in @($steamRoot, $discordRoot, $helperRoot, $foreignRoot, (Join-Path $discordRoot 'app-1.0.1'))) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}
$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$source = Join-Path $PSScriptRoot 'fixtures\known-app-client.cs'
function Build-Fixture([string]$OutputPath, [string]$Define) {
    & $compiler /nologo /target:winexe /optimize+ /reference:System.Windows.Forms.dll /reference:System.Drawing.dll "/define:$Define" "/out:$OutputPath" $source
    if ($LASTEXITCODE -ne 0) { throw "Fixture compilation failed: $OutputPath" }
}
Build-Fixture (Join-Path $steamRoot 'steam.exe') 'STEAM'
Copy-Item -LiteralPath (Join-Path $steamRoot 'steam.exe') -Destination (Join-Path $helperRoot 'steamwebhelper.exe')
Copy-Item -LiteralPath (Join-Path $steamRoot 'steam.exe') -Destination (Join-Path $foreignRoot 'steamwebhelper.exe')
Build-Fixture (Join-Path $discordRoot 'app-1.0.1\Discord.exe') 'DISCORD'
Build-Fixture (Join-Path $discordRoot 'Update.exe') 'UPDATER'
Build-Fixture (Join-Path $discordRoot 'Discord.exe') 'UPDATER,DISCORD_STUB'
[IO.File]::WriteAllText((Join-Path $fixtureRoot 'fixture.marker'), 'cpu-affinity-tool-known-app-test-v1')
$runner = Join-Path $fixtureRoot 'run.ps1'
$escapeTest = $resolvedTest.Replace("'", "''")
$escapeRoot = $fixtureRoot.Replace("'", "''")
$body = @"
`$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath '$($repoRoot.Replace("'", "''"))'
`$env:CAT_KNOWN_FIXTURE_DIR = '$escapeRoot'
`$env:CAT_KNOWN_RESULT_PATH = '$escapeRoot\result.json'
try {
    `$native = Start-Process -FilePath '$escapeTest' -ArgumentList @('--ignored','--exact','app::features::cpu_load::runtime::native_known_apps::native_known_app_launch_focus_affinity_cpu_ram','--nocapture') -WindowStyle Hidden -RedirectStandardOutput '$escapeRoot\test.log' -RedirectStandardError '$escapeRoot\test-stderr.log' -PassThru
    if (-not `$native.WaitForExit(90000)) { `$native.Kill(); `$native.WaitForExit(); `$testExit = 124 } else { `$native.Refresh(); `$testExit = `$native.ExitCode }
    if (`$null -eq `$testExit) { `$testExit = if (Test-Path -LiteralPath '$escapeRoot\result.json') { 0 } else { 1 } }
    [IO.File]::WriteAllText('$escapeRoot\exit.txt', [string]`$testExit)
    exit `$testExit
} catch {
    [IO.File]::WriteAllText('$escapeRoot\runner-error.txt', [string]`$_)
    [IO.File]::WriteAllText('$escapeRoot\exit.txt', '125')
    exit 125
}
"@
[IO.File]::WriteAllText($runner, $body)
if ($PrepareOnly) {
    [pscustomobject]@{ FixtureRoot = $fixtureRoot; Runner = $runner; TestBinary = $resolvedTest } | ConvertTo-Json -Compress
} else {
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $runner
    exit $LASTEXITCODE
}
