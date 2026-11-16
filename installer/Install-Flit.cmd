@echo off
setlocal EnableExtensions DisableDelayedExpansion
rem Release settings: set the repository before publishing this file.
set "FLIT_INSTALL_REPO=OWNER/REPO"
set "FLIT_INSTALL_TAG=v0.29.0"
set "FLIT_INSTALL_SHA256=cbd7d9f3646efacc5699aeacda3f82e56edeaa9dbf0091c9d7622f2eeed0800e"
set "FLIT_INSTALL_DIR=%~dp0"
rem Use Windows PowerShell modules even when launched from PowerShell 7.
set "PSModulePath=%SystemRoot%\System32\WindowsPowerShell\v1.0\Modules"

"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -Command ^
    "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; $tempDir = $null;" ^
    "try {" ^
    "    $file = Join-Path $env:FLIT_INSTALL_DIR 'FlitSetup-x64.exe';" ^
    "    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {" ^
    "        $repo = $env:FLIT_INSTALL_REPO; $tag = $env:FLIT_INSTALL_TAG;" ^
    "        if ($repo -eq 'OWNER/REPO') { throw 'Keep Install-Flit.cmd next to FlitSetup-x64.exe until the GitHub release is configured.' };" ^
    "        $url = 'https://github.com/{0}/releases/download/{1}/FlitSetup-x64.exe' -f $repo, $tag;" ^
    "        $tempDir = Join-Path ([IO.Path]::GetTempPath()) ('flit-install-' + [guid]::NewGuid().ToString('N'));" ^
    "        New-Item -ItemType Directory -Path $tempDir | Out-Null;" ^
    "        $file = Join-Path $tempDir 'FlitSetup-x64.exe';" ^
    "        Write-Host ('Downloading Flit Browser ' + $tag + '...');" ^
    "        Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $file;" ^
    "    };" ^
    "    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $env:FLIT_INSTALL_SHA256) { throw 'Installer SHA-256 mismatch. Download a fresh copy from the Flit release.' };" ^
    "    Write-Host 'Opening Flit Browser Setup...';" ^
    "    $setup = Start-Process -FilePath $file -PassThru; $setup.WaitForExit();" ^
    "    exit $setup.ExitCode;" ^
    "} catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 } finally {" ^
    "    if ($tempDir) {" ^
    "        try { [IO.File]::Delete((Join-Path $tempDir 'FlitSetup-x64.exe')); [IO.Directory]::Delete($tempDir) } catch { Write-Warning 'Could not remove the temporary download.' };" ^
    "    };" ^
    "}"
set "FLIT_INSTALL_RESULT=%ERRORLEVEL%"
if not "%FLIT_INSTALL_RESULT%"=="0" pause
exit /b %FLIT_INSTALL_RESULT%
