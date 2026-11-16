[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$IsccPath,
    [Parameter(Mandatory)][string]$BundleDir,
    [Parameter(Mandatory)][string]$OutputDir
)

$ErrorActionPreference = 'Stop'
$iscc = (Get-Item -LiteralPath $IsccPath).FullName
$payload = (Resolve-Path -LiteralPath $BundleDir).ProviderPath
$destination = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDir)
$output = Join-Path $destination 'FlitSetup-x64.exe'

if (Test-Path -LiteralPath $output) {
    throw "Choose an output folder without FlitSetup-x64.exe: $destination"
}
$required = @('Flit.exe','Flit.dll','libcef.dll','chrome_elf.dll','icudtl.dat','resources.pak',
    'v8_context_snapshot.bin','locales','LICENSE.txt','CREDITS.html','third-party-licenses')
foreach ($name in $required) {
    if (-not (Test-Path -LiteralPath (Join-Path $payload $name))) { throw "Missing bundle item: $name" }
}
if ((Test-Path -LiteralPath (Join-Path $payload 'User Data')) -or
    (Test-Path -LiteralPath (Join-Path $payload '.flit-browser-profile'))) {
    throw 'Use an application bundle, not a browser profile.'
}
if ($destination.Equals($payload, [StringComparison]::OrdinalIgnoreCase) -or
    $destination.StartsWith($payload.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The setup output must be outside the payload directory.'
}
if (-not (Test-Path -LiteralPath $destination -PathType Container)) {
    New-Item -ItemType Directory -Path $destination | Out-Null
}
& $iscc '--no-ide-signtools' "--define=BundleDir=$payload" "--output-dir=$destination" (Join-Path $PSScriptRoot 'Flit.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compiler exited with code $LASTEXITCODE." }
Get-Item -LiteralPath $output
