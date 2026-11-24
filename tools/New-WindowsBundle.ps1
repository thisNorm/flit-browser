[CmdletBinding(DefaultParameterSetName = 'Bundle')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Bundle')][string]$CefRuntime,
    [Parameter(Mandatory, ParameterSetName = 'Bundle')][string]$FlitDll,
    [Parameter(Mandatory, ParameterSetName = 'Bundle')][string]$Destination,
    [Parameter(ParameterSetName = 'Bundle')][string]$Version = '0.41.0',
    [Parameter(Mandatory, ParameterSetName = 'Bundle')][string]$ExactUiDir,
    [Parameter(Mandatory, ParameterSetName = 'Verify')][string]$VerifyStack
)

$ErrorActionPreference = 'Stop'
$stackReserve = 8MB

function Get-LauncherStack([string]$Path) {
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        $stream = $reader.BaseStream
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) {
            throw "Invalid Windows launcher: $Path"
        }
        $stream.Position = 0x3c
        $pe = [long]$reader.ReadInt32()
        if ($pe -lt 64 -or $pe + 24 + 88 -gt $stream.Length) {
            throw "Invalid PE header: $Path"
        }
        $stream.Position = $pe
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw "Expected an x64 Windows launcher: $Path"
        }
        $stream.Position = $pe + 20
        if ($reader.ReadUInt16() -lt 88) { throw "Missing PE stack fields: $Path" }
        $stream.Position = $pe + 24
        if ($reader.ReadUInt16() -ne 0x20b) { throw "Expected a PE32+ launcher: $Path" }
        $stream.Position = $pe + 24 + 72
        [pscustomobject]@{ Reserve = $reader.ReadUInt64(); Commit = $reader.ReadUInt64() }
    } finally {
        $reader.Dispose()
    }
}

function Assert-LauncherStack([string]$Path) {
    $stack = Get-LauncherStack $Path
    if ($stack.Reserve -ne $stackReserve) {
        throw "Flit.exe needs an 8 MiB stack reserve for Exact UI (found $($stack.Reserve) bytes). Rebuild with New-WindowsBundle.ps1."
    }
    $stack
}

if ($PSCmdlet.ParameterSetName -eq 'Verify') {
    Assert-LauncherStack (Get-Item -LiteralPath $VerifyStack).FullName
    return
}

$editbin = (Get-Command editbin.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
if (!$editbin) {
    $vswhere = Join-Path ([Environment]::GetEnvironmentVariable('ProgramFiles(x86)')) 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (Test-Path -LiteralPath $vswhere) {
        $editbin = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\editbin.exe' | Select-Object -First 1
    }
}
if (!$editbin) { throw 'Install Visual Studio Build Tools with MSVC x64, or run this script from Developer PowerShell with editbin.exe on PATH.' }

$cef = (Resolve-Path -LiteralPath $CefRuntime).ProviderPath
$dll = (Get-Item -LiteralPath $FlitDll).FullName
$exact = (Resolve-Path -LiteralPath $ExactUiDir).ProviderPath
$exactDll = Join-Path $exact 'FlitExact.dll'
$exactNotices = Join-Path $exact 'third-party-licenses/ExpoExact'
foreach ($path in @($exactDll) + @('LICENSE', 'NOTICE', 'DEPENDENCIES.txt', 'ExactRevision.txt' | ForEach-Object { Join-Path $exactNotices $_ })) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing Exact UI build item: $path" }
}
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
$launcher = Join-Path $destinationPath 'Flit.exe'
& (Join-Path $PSScriptRoot 'Set-FlitIcon.ps1') -BootstrapPath (Join-Path $cef 'bootstrap.exe') -OutputPath $launcher | Out-Null
$originalStack = Get-LauncherStack $launcher
& $editbin '/NOLOGO' "/STACK:$stackReserve,$($originalStack.Commit)" $launcher | Out-Null
if ($LASTEXITCODE -ne 0) { throw "editbin exited with code $LASTEXITCODE." }
$newStack = Assert-LauncherStack $launcher
if ($newStack.Commit -ne $originalStack.Commit) { throw 'editbin changed the original stack commit size.' }
Copy-Item -LiteralPath $dll -Destination (Join-Path $destinationPath 'Flit.dll')
Copy-Item -LiteralPath $exactDll -Destination $destinationPath
$thirdParty = Join-Path $destinationPath 'third-party-licenses'
New-Item -ItemType Directory -Path $thirdParty | Out-Null
Copy-Item -LiteralPath $exactNotices -Destination $thirdParty -Recurse
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
Get-Item -LiteralPath $launcher
