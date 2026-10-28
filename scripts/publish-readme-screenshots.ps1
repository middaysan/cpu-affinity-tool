[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$CaptureDirectory,
    [string]$Python = 'python'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'The lossless README compositor uses Windows System.Drawing.' }
$repoRoot = Split-Path -Parent $PSScriptRoot
$captureRoot = [IO.Path]::GetFullPath($CaptureDirectory)
$config = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'fixtures/readme-screenshots.json') | ConvertFrom-Json
& $Python (Join-Path $PSScriptRoot 'check-readme-screenshots.py') --capture $captureRoot
if ($LASTEXITCODE -ne 0) { throw 'Screenshot verification failed; assets were not replaced.' }
$report = Get-Content -Raw -LiteralPath (Join-Path $captureRoot 'capture.json') | ConvertFrom-Json
$report.captured_utc = ([DateTimeOffset]$report.captured_utc).ToUniversalTime().ToString('o')
$staging = Join-Path $repoRoot ('target/ss-publish-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $staging | Out-Null
Add-Type -AssemblyName System.Drawing
$images = @()
$graphics = $null
$canvas = $null
$font = $null
$textBrush = $null
$separatorBrush = $null
try {
    $scenes = @($config.hero.scenes | ForEach-Object {
        $sceneId = $_
        $config.scenes | Where-Object { $_.id -eq $sceneId }
    })
    foreach ($scene in $scenes) {
        $images += [Drawing.Image]::FromFile((Join-Path $captureRoot $scene.file))
    }
    if ($images.Count -ne 3) { throw 'The lead composition requires exactly three actual frames.' }
    $padding = [int]$config.hero.padding
    $gutter = [int]$config.hero.gutter
    $header = [int]$config.hero.header
    $width = 2 * $padding + 2 * $gutter + ($images | Measure-Object -Property Width -Sum).Sum
    $height = 2 * $padding + $header + ($images | Measure-Object -Property Height -Maximum).Maximum
    $canvas = [Drawing.Bitmap]::new([int]$width, [int]$height, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [Drawing.Graphics]::FromImage($canvas)
    $graphics.Clear([Drawing.ColorTranslator]::FromHtml('#0d1117'))
    $font = [Drawing.Font]::new('Segoe UI', 25, [Drawing.FontStyle]::Bold, [Drawing.GraphicsUnit]::Pixel)
    $textBrush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml('#c9d1d9'))
    $separatorBrush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml('#697785'))
    $columns = @()
    $left = $padding
    for ($index = 0; $index -lt 3; $index++) {
        $image = $images[$index]
        $graphics.TextRenderingHint = [Drawing.Text.TextRenderingHint]::AntiAliasGridFit
        $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceOver
        $graphics.DrawString($scenes[$index].label, $font, $textBrush, [single]$left, [single]($padding + 5))
        $columns += @{ scene = $scenes[$index].id; file = $scenes[$index].file; x = $left;
            y = $padding + $header; width = $image.Width; height = $image.Height }
        $left += $image.Width
        if ($index -lt 2) {
            $graphics.FillRectangle($separatorBrush, [int]($left + ($gutter - $config.hero.separator) / 2),
                $padding, [int]$config.hero.separator, [int]($height - 2 * $padding))
            $left += $gutter
        }
    }
    $graphics.Dispose()
    $graphics = $null
    # Copy straight-alpha pixels after drawing the exterior. GDI+ DrawImage would
    # round translucent renderer pixels through its premultiplied-alpha pipeline.
    for ($index = 0; $index -lt 3; $index++) {
        $image = $images[$index]
        $column = $columns[$index]
        $sourceBits = $null
        $targetBits = $null
        try {
            $sourceBits = $image.LockBits([Drawing.Rectangle]::new(0, 0, $image.Width, $image.Height),
                [Drawing.Imaging.ImageLockMode]::ReadOnly, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $targetBits = $canvas.LockBits([Drawing.Rectangle]::new($column.x, $column.y, $image.Width, $image.Height),
                [Drawing.Imaging.ImageLockMode]::WriteOnly, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $row = [byte[]]::new($image.Width * 4)
            for ($y = 0; $y -lt $image.Height; $y++) {
                [Runtime.InteropServices.Marshal]::Copy([IntPtr]::Add($sourceBits.Scan0, $y * $sourceBits.Stride), $row, 0, $row.Length)
                [Runtime.InteropServices.Marshal]::Copy($row, 0, [IntPtr]::Add($targetBits.Scan0, $y * $targetBits.Stride), $row.Length)
            }
        } finally {
            if ($sourceBits) { $image.UnlockBits($sourceBits) }
            if ($targetBits) { $canvas.UnlockBits($targetBits) }
        }
    }
    $heroPath = Join-Path $staging 'screenshot.png'
    $canvas.Save($heroPath, [Drawing.Imaging.ImageFormat]::Png)
    $report | Add-Member -NotePropertyName hero -NotePropertyValue @{
        file = 'assets/screenshot.png'; width = [int]$width; height = [int]$height;
        sha256 = (Get-FileHash -LiteralPath $heroPath -Algorithm SHA256).Hash.ToLowerInvariant();
        columns = $columns; gutter = $gutter; separator = [int]$config.hero.separator;
        method = 'Three original PNG frames, unscaled; labels and separators outside application pixels'
    } -Force
    # Publish only the checked PNG whitelist and sanitized provenance, never logs/tickets/keys.
    $destination = Join-Path $repoRoot 'assets/screenshots'
    foreach ($scene in $config.scenes) {
        Copy-Item -LiteralPath (Join-Path $captureRoot $scene.file) -Destination (Join-Path $destination $scene.file) -Force
    }
    Copy-Item -LiteralPath $heroPath -Destination (Join-Path $repoRoot 'assets/screenshot.png') -Force
    [IO.File]::WriteAllText((Join-Path $destination 'capture.json'), ($report | ConvertTo-Json -Depth 50), [Text.UTF8Encoding]::new($false))
} finally {
    if ($graphics) { $graphics.Dispose() }
    if ($canvas) { $canvas.Dispose() }
    if ($font) { $font.Dispose() }
    if ($textBrush) { $textBrush.Dispose() }
    if ($separatorBrush) { $separatorBrush.Dispose() }
    foreach ($image in $images) { $image.Dispose() }
}
& $Python (Join-Path $PSScriptRoot 'check-readme-screenshots.py')
if ($LASTEXITCODE -ne 0) { throw 'Published screenshot verification failed.' }
Write-Host 'Published eight unchanged native PNGs, their provenance, and the three-view lead image.'
