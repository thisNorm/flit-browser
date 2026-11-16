# Regenerate the Windows icon from the supplied Flit PNG using installed FFmpeg.
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$PngPath
)

$ErrorActionPreference = 'Stop'
$source = (Get-Item -LiteralPath $PngPath).FullName
$ffmpeg = (Get-Command ffmpeg -CommandType Application).Source
$assets = Join-Path (Split-Path -Parent $PSScriptRoot) 'assets'
$null = New-Item -ItemType Directory -Path $assets -Force

$sizes = 16, 20, 24, 32, 40, 48, 64, 96, 128, 256
$labels = $sizes | ForEach-Object { "[s$_]" }
$crop = "crop='min(iw,ih)':'min(iw,ih)'"
$filters = @("[0:v]$crop,split=$($sizes.Count)$($labels -join '')")
$streams = @()
foreach ($i in 0..($sizes.Count - 1)) {
    $n = $sizes[$i]
    # Keep native 32-bit DIBs for the small Windows icons; compress the largest frame.
    $codec, $pixels = if ($n -eq 256) { 'png', 'rgba' } else { 'bmp', 'bgra' }
    $filter = "[s$n]scale=${n}:${n}:flags=lanczos,format=$pixels"
    $filters += if ($n -eq 256) { "$filter,split=2[i$n][preview]" } else { "$filter[i$n]" }
    $streams += '-map', "[i$n]", "-c:v:$i", $codec
}

$tempName = Join-Path $assets ('.flit-icon-' + [guid]::NewGuid().ToString('N'))
$tempIco = "$tempName.ico"
$tempPreview = "$tempName.png"
try {
    & $ffmpeg -hide_banner -loglevel error -nostdin -n -i $source `
        -filter_complex ($filters -join ';') @streams -f ico $tempIco `
        -map '[preview]' -c:v png -frames:v 1 -update 1 $tempPreview
    if ($LASTEXITCODE -ne 0) { throw "FFmpeg could not convert the icon (exit $LASTEXITCODE)." }

    Move-Item -LiteralPath $tempPreview -Destination (Join-Path $assets 'Flit-preview.png') -Force
    Move-Item -LiteralPath $tempIco -Destination (Join-Path $assets 'Flit.ico') -Force
} finally {
    [IO.File]::Delete($tempIco)
    [IO.File]::Delete($tempPreview)
}
