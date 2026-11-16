#Requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][ValidatePattern('\A(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\z')][string]$Version,
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$Cef,
    [Parameter(Mandatory)][string]$PrivateKeyPath,
    [Parameter(Mandatory)][string]$OutputDir
)

$ErrorActionPreference = 'Stop'
$destination = Get-Item -LiteralPath $OutputDir -Force
if ($destination -isnot [IO.DirectoryInfo]) { throw 'OutputDir must be an existing directory.' }
$manifestPath = Join-Path $destination.FullName 'flit-update.json'
$signaturePath = Join-Path $destination.FullName 'flit-update.json.sig'
foreach ($path in @($manifestPath, $signaturePath)) {
    if (Test-Path -LiteralPath $path) { throw "Refusing to overwrite: $path" }
}

$setup = Get-Item -LiteralPath $InstallerPath -Force
if ($setup -isnot [IO.FileInfo] -or $setup.Name -cne 'FlitSetup-x64.exe') {
    throw 'InstallerPath must point to FlitSetup-x64.exe.'
}
$installer = [IO.File]::Open($setup.FullName, 'Open', 'Read', 'Read')
try {
    $productVersion = ([string]$setup.VersionInfo.ProductVersion).Trim()
    if ($productVersion -cne $Version) {
        throw "Installer ProductVersion '$productVersion' does not match feed version '$Version'."
    }
    $sha256 = (Get-FileHash -InputStream $installer -Algorithm SHA256).Hash.ToLowerInvariant()
} finally {
    $installer.Dispose()
}

$json = [ordered]@{ version = $Version; sha256 = $sha256; cef = $Cef } | ConvertTo-Json -Compress
$manifest = [Text.UTF8Encoding]::new($false).GetBytes($json)

$key = Get-Item -LiteralPath $PrivateKeyPath -Force
if ($key -isnot [IO.FileInfo]) { throw 'PrivateKeyPath must point to a PEM file.' }
$rsa = [Security.Cryptography.RSA]::Create()
try {
    $rsa.ImportFromPem([IO.File]::ReadAllText($key.FullName))
    $public = $rsa.ExportParameters($false)
    # AQAB is the base64 encoding of the unsigned big-endian exponent 65537.
    if ($rsa.KeySize -notin 2048,3072,4096 -or [Convert]::ToBase64String($public.Exponent) -cne 'AQAB') {
        throw 'Use a 2048-, 3072-, or 4096-bit RSA private key with public exponent 65537.'
    }
    $signature = $rsa.SignData($manifest, [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pkcs1)
} finally {
    $rsa.Dispose()
}

$created = @()
$assets = [ordered]@{ $manifestPath = $manifest; $signaturePath = $signature }
try {
    foreach ($path in $assets.Keys) {
        $file = [IO.File]::Open($path, 'CreateNew', 'Write', 'None')
        try {
            $created += $path
            $file.Write($assets[$path], 0, $assets[$path].Length)
        } finally {
            $file.Dispose()
        }
    }
} catch {
    foreach ($path in $created) { Remove-Item -LiteralPath $path -ErrorAction SilentlyContinue }
    throw
}

Write-Output "FLIT_UPDATE_RSA_MODULUS=$([Convert]::ToBase64String($public.Modulus))"
