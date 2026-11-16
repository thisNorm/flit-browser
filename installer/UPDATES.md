# Flit Browser automatic updates

Flit has no live update feed until a build sets both `FLIT_UPDATE_REPOSITORY`
and `FLIT_UPDATE_RSA_MODULUS`. A configured browser checks the public GitHub
repository's latest release in the background at most once a day. It verifies
the feed signature before trusting the version or hash. For a newer version it
downloads the installer and checks its SHA-256 before running it. Failed
verification leaves the installed browser in place. A staged update applies
after a normal exit, when the browser is no longer using the runtime files. If
staging finishes after exit, it can apply on the next normal exit. There is no
persistent update service.

## Configure the release build

Use PowerShell 7.4 or newer on Windows. The maintainer supplies an existing RSA
private key from a restricted location outside the source tree and release
folder. The helper reads an unencrypted PEM file containing one PKCS#8
(`PRIVATE KEY`) or PKCS#1 (`RSA PRIVATE KEY`) key. Use 2048, 3072, or 4096 bits and
public exponent 65537. Keep the PEM out of commits, build inputs, and release
assets. Only the public modulus is built into the browser.

In the shell that will build Flit, derive the base64 public modulus (the raw
unsigned big-endian modulus, not a PEM or a certificate) and set the repository:

```powershell
$keyPath = 'D:\Secure\flit-update-private.pem'
$rsa = [System.Security.Cryptography.RSA]::Create()
try {
    $rsa.ImportFromPem([IO.File]::ReadAllText($keyPath))
    $env:FLIT_UPDATE_RSA_MODULUS = [Convert]::ToBase64String($rsa.ExportParameters($false).Modulus)
} finally {
    $rsa.Dispose()
}
$env:FLIT_UPDATE_REPOSITORY = 'OWNER/REPO'
```

Replace `OWNER/REPO` with the public release repository. Set both variables
before building the browser; changing them later requires a rebuild. Existing
installations trust the public key they were built with, so switching to a new
key requires a planned transition for those installations.

## Build and publish a release

1. Bump the browser and installer product versions together (for example,
   `0.2.0`), and keep the Inno Setup `AppId` at
   `{7B05A21B-1FC7-4A43-B8C5-2C97474A79FA}` so the setup upgrades the existing
   installation. Follow the [installer build instructions](README.md) to
   package the whole runtime. The CEF bootstrap (`Flit.exe`), `libcef.dll`,
   `chrome_elf.dll`, `Flit.dll`, and CEF resources and locales must come from
   the same compatible build. Ship the full installer for every update.
2. Run the helper against the final installer. If Windows Authenticode signing
   is added in the future, sign and timestamp the EXE in a separate, explicit
   step **before** generating the feed. A feed signature does not sign the EXE
   for Windows. Any later change to the EXE invalidates the feed's hash.

   ```powershell
   .\New-UpdateFeed.ps1 -InstallerPath 'C:\Build\release\FlitSetup-x64.exe' `
       -Version '0.2.0' -Cef '156.0.26+g8ea0c73' `
       -PrivateKeyPath $keyPath -OutputDir 'C:\Build\release'
   ```

   The output directory must already exist and contain neither feed file. The
   helper checks the EXE's Windows product version against `-Version` and
   prints the signing key's public modulus; confirm it matches the release
   build's `FLIT_UPDATE_RSA_MODULUS`.
3. Attach all three assets to the same GitHub release before publishing it as
   the public latest release (not a draft or prerelease). Use the tag `vVERSION`,
   for example `v0.2.0`, and these exact asset names:

   | Asset | Contents |
   | --- | --- |
   | `FlitSetup-x64.exe` | The complete Windows x64 installer. |
   | `flit-update.json` | UTF-8 without BOM: `version` is `MAJOR.MINOR.PATCH`, `sha256` is the installer's 64-character hex SHA-256, and `cef` is the CEF version for reference. |
   | `flit-update.json.sig` | Raw binary RSA PKCS#1 v1.5 signature with SHA-256 over the exact `flit-update.json` bytes. |

   Upload the generated files as-is. Reformatting JSON or changing its line
   endings invalidates the signature. Publish corrections under a new, higher
   version so installed browsers can detect them.
