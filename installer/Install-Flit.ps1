[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^[a-zA-Z0-9][a-zA-Z0-9-]*/[a-zA-Z0-9_.-]+$')]
    [string]$Repository,
    [switch]$Silent
)

$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'This installer requires Windows.'
}
if ($Repository.Split('/')[-1] -in '.', '..') { throw 'Enter the GitHub repository as OWNER/REPO.' }

$ProgressPreference = 'SilentlyContinue'
$headers = @{ Accept = 'application/vnd.github+json' }
$release = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$Repository/releases/latest" -Headers $headers
$asset = @($release.assets | Where-Object { $_.name -ceq 'FlitSetup-x64.exe' -and $_.state -eq 'uploaded' })
if ($asset.Count -ne 1 -or $asset[0].digest -notmatch '^sha256:([0-9a-f]{64})$') {
    throw 'The latest release does not contain a Flit installer with a GitHub SHA-256 digest.'
}
$expected = $Matches[1]
$url = [uri]$asset[0].browser_download_url
$pathPrefix = "/$Repository/releases/download/"
if ($url.Scheme -ne 'https' -or $url.Host -ne 'github.com' -or
    -not $url.AbsolutePath.StartsWith($pathPrefix, [StringComparison]::OrdinalIgnoreCase) -or
    $url.Segments[-1] -cne 'FlitSetup-x64.exe') {
    throw 'Unexpected GitHub release download URL.'
}

$work = Join-Path ([IO.Path]::GetTempPath()) ('flit-install-' + [guid]::NewGuid().ToString('N'))
$installer = Join-Path $work 'FlitSetup-x64.exe'
New-Item -ItemType Directory -Path $work | Out-Null
try {
    Write-Host "Downloading Flit Browser $($release.tag_name)..."
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $installer
    if ((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash -ne $expected) {
        throw 'The downloaded installer did not match the GitHub release digest.'
    }

    $setupOptions = @{ FilePath = $installer; PassThru = $true; Wait = $true }
    if ($Silent) {
        $setupOptions.ArgumentList = @('/VERYSILENT','/SUPPRESSMSGBOXES','/SP-','/NORESTART','/RESTARTEXITCODE=3010')
    }
    $process = Start-Process @setupOptions
    if ($process.ExitCode -eq 3010) { Write-Host 'Flit installed. Windows reports that a restart is needed.' }
    elseif ($process.ExitCode -ne 0) { throw "Flit Setup exited with code $($process.ExitCode)." }
    else { Write-Host 'Flit Browser is available in the Start menu.' }
} finally {
    if (Test-Path -LiteralPath $installer -PathType Leaf) { Remove-Item -LiteralPath $installer -Force }
    Remove-Item -LiteralPath $work -ErrorAction SilentlyContinue
}
