[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$CefRuntime,
    [Parameter(Mandatory)][string]$FlitDll,
    [Parameter(Mandatory)][string]$Destination,
    [string]$Version = '0.29.0'
)

$ErrorActionPreference = 'Stop'
$cef = (Resolve-Path -LiteralPath $CefRuntime).ProviderPath
$dll = (Get-Item -LiteralPath $FlitDll).FullName
$destinationPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Destination)
if (Test-Path -LiteralPath $destinationPath) {
    throw 'Choose a new bundle directory. Existing files are preserved.'
}
$runtimeFiles = @(
    'chrome_elf.dll', 'chrome_100_percent.pak', 'chrome_200_percent.pak',
    'libcef.dll', 'icudtl.dat', 'resources.pak', 'd3dcompiler_47.dll', 'dxcompiler.dll',
    'v8_context_snapshot.bin', 'vk_swiftshader.dll', 'vk_swiftshader_icd.json', 'vulkan-1.dll',
    'LICENSE.txt', 'CREDITS.html', 'locales'
)
foreach ($item in $runtimeFiles + @('bootstrap.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $cef $item))) { throw "Missing CEF runtime item: $item" }
}

New-Item -ItemType Directory -Path $destinationPath | Out-Null
& (Join-Path $PSScriptRoot 'Set-FlitIcon.ps1') -BootstrapPath (Join-Path $cef 'bootstrap.exe') -OutputPath (Join-Path $destinationPath 'Flit.exe') | Out-Null
Copy-Item -LiteralPath $dll -Destination (Join-Path $destinationPath 'Flit.dll')
foreach ($item in $runtimeFiles) {
    Copy-Item -LiteralPath (Join-Path $cef $item) -Destination $destinationPath -Recurse
}
@"
Flit Browser $Version - Windows 11 x64 development preview

Extract the whole folder, then run Flit.exe. Keep the DLL and resources next to it.

Flit stores its Chromium profile under %LOCALAPPDATA%\Flit Browser\User Data.
You can choose a different absolute path with --flit-profile=PATH.
The folder must be empty or already initialized by Flit. Other browsers' profiles are not supported.

For local MCP, configure Flit.exe with the single argument --flit-agent and stdio transport.
Flit Vault supports selected HTTPS password forms. The preview has no connected update channel.
Review LICENSE.txt and CREDITS.html for CEF and the bundled Chromium components.
"@ | Set-Content -LiteralPath (Join-Path $destinationPath 'README.txt') -Encoding UTF8
Get-Item -LiteralPath (Join-Path $destinationPath 'Flit.exe')
