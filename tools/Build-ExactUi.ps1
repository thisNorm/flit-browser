[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Destination,
    [string]$BuiltDll,
    [string]$TargetDir
)

$ErrorActionPreference = 'Stop'
$native = Join-Path $PSScriptRoot '../native-ui'
$upstream = Join-Path $native 'upstream'
$app = Join-Path $native 'app'
$manifest = Join-Path $app 'Cargo.toml'
$revisionFile = Join-Path $native 'ExactRevision.txt'
$revision = (Get-Content -Raw -LiteralPath $revisionFile).Trim().ToLowerInvariant()
if ($revision -notmatch '\A[0-9a-f]{40}\z') { throw "Invalid Exact commit in $revisionFile." }
$destinationPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Destination)
if (Test-Path -LiteralPath $destinationPath) { throw "Exact UI output already exists: $destinationPath" }

function Invoke-ExactGit {
    $gitContext = @{}
    foreach ($name in 'GIT_DIR','GIT_WORK_TREE','GIT_INDEX_FILE','GIT_COMMON_DIR','GIT_OBJECT_DIRECTORY','GIT_ALTERNATE_OBJECT_DIRECTORIES') {
        $gitContext[$name] = [Environment]::GetEnvironmentVariable($name)
        [Environment]::SetEnvironmentVariable($name, [NullString]::Value)
    }
    try {
        $result = & git.exe --no-optional-locks -C $upstream @args
        if ($LASTEXITCODE -ne 0) { throw "Git failed in the Exact checkout: $($args[0])." }
        $result
    } finally {
        foreach ($name in $gitContext.Keys) {
            $prior = $gitContext[$name]
            if ($null -eq $prior) { $prior = [NullString]::Value }
            [Environment]::SetEnvironmentVariable($name, $prior)
        }
    }
}

if (-not (Test-Path -LiteralPath $upstream -PathType Container)) {
    New-Item -ItemType Directory -Path $upstream | Out-Null
    Invoke-ExactGit init --quiet --template= | Out-Null
    Invoke-ExactGit fetch --quiet --depth=1 --no-tags --no-recurse-submodules https://github.com/expo/exact.git $revision | Out-Null
    Invoke-ExactGit -c core.hooksPath=NUL checkout --quiet --detach $revision | Out-Null
}
if (-not (Test-Path -LiteralPath (Join-Path $upstream '.git'))) {
    throw "Expected a standalone pinned Exact checkout at $upstream."
}
if ((Invoke-ExactGit rev-parse --verify HEAD).Trim() -cne $revision) {
    throw "Expected Exact commit $revision at $upstream. The existing checkout was preserved."
}
if (Invoke-ExactGit status --porcelain --untracked-files=no) {
    throw "Tracked files in the Exact checkout differ from $revision."
}

$triple = 'x86_64-pc-windows-msvc'
if ($BuiltDll) {
    $dll = Get-Item -LiteralPath $BuiltDll
} else {
    if (!(& rustc.exe -vV | Where-Object { $_ -ceq "host: $triple" })) {
        throw 'Use an x64 MSVC Rust host toolchain to build FlitExact.dll.'
    }
    $programFiles32 = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
    $vswhere = Join-Path $programFiles32 'Microsoft Visual Studio/Installer/vswhere.exe'
    $compiler = Get-Command cl.exe -CommandType Application -ErrorAction SilentlyContinue
    if (!$compiler -and (Test-Path -LiteralPath $vswhere)) {
        $compiler = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\cl.exe' | Select-Object -First 1
    }
    if (!$compiler) { throw 'Install Visual Studio Build Tools with MSVC x64 and a Windows SDK, or use an x64 Developer PowerShell.' }

    $fxc = (Get-Command fxc.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (!$fxc) {
        $sdk = Join-Path $programFiles32 'Windows Kits/10/bin'
        $fxc = Get-ChildItem -LiteralPath $sdk -Directory -ErrorAction SilentlyContinue |
            Where-Object Name -Match '^\d+\.\d+\.\d+\.\d+$' |
            Sort-Object { [version]$_.Name } -Descending |
            ForEach-Object { Join-Path $_.FullName 'x64/fxc.exe' } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
            Select-Object -First 1
    }
    if (!$fxc) { throw 'Exact needs fxc.exe. Install the Windows 10 or 11 SDK with its x64 shader compiler.' }
    if (!$TargetDir) {
        $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $app 'target' }
    }
    $targetPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($TargetDir)
    $savedPath = $env:Path
    $savedFlags = $env:RUSTFLAGS
    $savedEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
    $savedTargetFlags = $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS
    $savedTarget = $env:CARGO_BUILD_TARGET
    $savedHermes = $env:HERMES_LEAN_SYS_OFFLINE
    try {
        $env:Path = (Split-Path -Parent $fxc) + ';' + $savedPath
        $env:HERMES_LEAN_SYS_OFFLINE = '1'
        $env:CARGO_BUILD_TARGET = $null
        if ($null -ne $savedEncodedFlags) {
            if ($savedEncodedFlags -notmatch '\+crt-static' -or $savedEncodedFlags -match '-crt-static') {
                $env:CARGO_ENCODED_RUSTFLAGS = (@($savedEncodedFlags, '-Ctarget-feature=+crt-static') -ne '') -join [char]31
            }
        } elseif ($null -ne $savedFlags) {
            if ($savedFlags -notmatch '\+crt-static' -or $savedFlags -match '-crt-static') {
                $env:RUSTFLAGS = ($savedFlags + ' -C target-feature=+crt-static').Trim()
            }
        } else {
            if ($savedTargetFlags -notmatch '\+crt-static' -or $savedTargetFlags -match '-crt-static') {
                $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = ($savedTargetFlags + ' -C target-feature=+crt-static').Trim()
            }
        }
        & cargo.exe build --locked --release --jobs 2 --target-dir $targetPath --manifest-path $manifest
        if ($LASTEXITCODE -ne 0) { throw "Exact release build failed (Cargo exit $LASTEXITCODE)." }
    } finally {
        $env:Path = $savedPath
        $env:RUSTFLAGS = $savedFlags
        $env:CARGO_ENCODED_RUSTFLAGS = $savedEncodedFlags
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = $savedTargetFlags
        $env:CARGO_BUILD_TARGET = $savedTarget
        $env:HERMES_LEAN_SYS_OFFLINE = $savedHermes
    }
    $dll = Get-Item -LiteralPath (Join-Path $targetPath 'release/flit_exact_ui.dll')
}
if ($dll -isnot [IO.FileInfo] -or $dll.Extension -ine '.dll' -or $dll.Length -eq 0) {
    throw 'Pass the built Flit Exact release DLL as -BuiltDll.'
}

$graph = & cargo.exe metadata --locked --format-version 1 --filter-platform $triple --manifest-path $manifest |
    ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or !$graph.resolve.root) { throw 'Could not resolve the locked Exact licensing graph.' }
$nodes = @{}
foreach ($node in $graph.resolve.nodes) { $nodes[$node.id] = $node }
$runtime = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$pending = [Collections.Generic.Stack[string]]::new()
$pending.Push($graph.resolve.root)
while ($pending.Count -ne 0) {
    $id = $pending.Pop()
    if (!$runtime.Add($id)) { continue }
    foreach ($edge in $nodes[$id].deps) {
        if ($edge.dep_kinds | Where-Object { $null -eq $_.kind }) { $pending.Push($edge.pkg) }
    }
}
$null = $runtime.Remove($graph.resolve.root)

$notices = Join-Path $destinationPath 'third-party-licenses/ExpoExact'
New-Item -ItemType Directory -Path $notices | Out-Null
foreach ($name in 'LICENSE', 'NOTICE') {
    Copy-Item -LiteralPath (Join-Path $upstream $name) -Destination $notices
}
Copy-Item -LiteralPath $revisionFile -Destination $notices
$localRoot = (Resolve-Path -LiteralPath $upstream).ProviderPath.TrimEnd('\') + '\'
$licenseName = '^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)([-_.].*)?$'
$inventory = foreach ($crate in ($graph.packages | Where-Object { $runtime.Contains($_.id) } | Sort-Object name, version)) {
    $dir = Split-Path -Parent $crate.manifest_path
    $files = @(Get-ChildItem -LiteralPath $dir -File | Where-Object Name -Match $licenseName)
    if ($crate.license_file) {
        $file = $crate.license_file
        if (![IO.Path]::IsPathRooted($file)) { $file = Join-Path $dir $file }
        $files += Get-Item -LiteralPath $file
    }
    if (!$files -and $dir.StartsWith($localRoot, [StringComparison]::OrdinalIgnoreCase)) {
        $ancestor = Split-Path -Parent $dir
        while ($ancestor.StartsWith($localRoot, [StringComparison]::OrdinalIgnoreCase) -and !$files) {
            $files = @(Get-ChildItem -LiteralPath $ancestor -File | Where-Object Name -Match $licenseName)
            $ancestor = Split-Path -Parent $ancestor
        }
    }
    $license = if ($crate.license) { $crate.license } else { 'see notice files' }
    "$($crate.name) $($crate.version) - $license"
    if ($files) {
        $crateNotices = Join-Path $notices "$($crate.name)-$($crate.version)"
        New-Item -ItemType Directory -Path $crateNotices -Force | Out-Null
        foreach ($file in ($files | Sort-Object FullName -Unique)) {
            Copy-Item -LiteralPath $file.FullName -Destination $crateNotices
        }
    }
}
@('Expo Exact Windows runtime dependencies', "Exact source revision $revision", '') + $inventory |
    Set-Content -LiteralPath (Join-Path $notices 'DEPENDENCIES.txt') -Encoding UTF8
Copy-Item -LiteralPath $dll.FullName -Destination (Join-Path $destinationPath 'FlitExact.dll')
Get-Item -LiteralPath (Join-Path $destinationPath 'FlitExact.dll')
