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
$checksum = Join-Path $destination 'FlitSetup-x64.exe.sha256'
$launcher = Join-Path $destination 'Install-Flit.cmd'

foreach ($path in @($output, $checksum, $launcher)) {
    if (Test-Path -LiteralPath $path) { throw "Release output already exists: $path" }
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
$template = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'Install-Flit.cmd'))
$tagLine = [regex]'(?m)^set "FLIT_INSTALL_TAG=[^"\r\n]*"(?=\r?$)'
$hashLine = [regex]'(?m)^set "FLIT_INSTALL_SHA256=[^"\r\n]*"(?=\r?$)'
if ($tagLine.Matches($template).Count -ne 1 -or $hashLine.Matches($template).Count -ne 1) {
    throw 'Install-Flit.cmd must contain one version setting and one SHA-256 setting.'
}
if (-not (Test-Path -LiteralPath $destination -PathType Container)) {
    New-Item -ItemType Directory -Path $destination | Out-Null
}
& $iscc '--no-ide-signtools' "--define=BundleDir=$payload" "--output-dir=$destination" (Join-Path $PSScriptRoot 'Flit.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compiler exited with code $LASTEXITCODE." }

$setup = Get-Item -LiteralPath $output
$version = $setup.VersionInfo.ProductVersion.Trim()
if ($version -notmatch '\A[0-9]+\.[0-9]+\.[0-9]+\z') { throw "Invalid installer product version: $version" }
$hash = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
$script = $tagLine.Replace($template, ('set "FLIT_INSTALL_TAG=v{0}"' -f $version))
$script = $hashLine.Replace($script, ('set "FLIT_INSTALL_SHA256={0}"' -f $hash))
[IO.File]::WriteAllText($launcher, $script, [Text.Encoding]::ASCII)
[IO.File]::WriteAllText($checksum, "$hash  $($setup.Name)" + [Environment]::NewLine, [Text.Encoding]::ASCII)
$setup
