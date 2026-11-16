$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Read-FlitVersion([string]$value) {
    if ($value -notmatch '^(0|[1-9][0-9]{0,3})\.(0|[1-9][0-9]{0,3})\.(0|[1-9][0-9]{0,3})$') {
        throw 'Invalid update version.'
    }
    return [version]$value
}

function Write-FlitText([string]$path, [string]$value) {
    $temporary = "$path.new"
    [IO.File]::WriteAllText($temporary, $value, [Text.UTF8Encoding]::new($false))
    if ([IO.File]::Exists($path)) {
        $backup = "$path.old"
        [IO.File]::Delete($backup)
        [IO.File]::Replace($temporary, $path, $backup)
        [IO.File]::Delete($backup)
    }
    else { [IO.File]::Move($temporary, $path) }
}

function Confirm-FlitInstall([string]$exe, [version]$current) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(
        'Software\Microsoft\Windows\CurrentVersion\Uninstall\{7B05A21B-1FC7-4A43-B8C5-2C97474A79FA}_is1')
    if ($null -eq $key) { throw 'Flit must be installed before it can update.' }
    try {
        $dir = [string]$key.GetValue('InstallLocation', '')
        if (!$dir -or [IO.Path]::GetFullPath((Join-Path $dir 'Flit.exe')) -ine $exe -or
            (Read-FlitVersion $key.GetValue('DisplayVersion', '')) -ne $current) {
            throw 'The registered Flit installation does not match this running version.'
        }
    } finally { $key.Dispose() }
}

function Get-FlitAsset([string]$uri, [string]$destination, [long]$limit) {
    $request = [Net.HttpWebRequest]::Create($uri)
    $request.AllowAutoRedirect = $false
    $request.Timeout = 15000
    $request.ReadWriteTimeout = 15000
    $request.UserAgent = 'FlitBrowserUpdate'
    $response = $null
    try {
        for ($redirects = 0; $redirects -le 3; $redirects++) {
            $response = $request.GetResponse()
            if ([int]$response.StatusCode -eq 200) { break }
            if ([int]$response.StatusCode -notin 301,302,303,307,308 -or $redirects -eq 3) {
                throw 'Unexpected release server response.'
            }
            $next = [uri]::new($request.RequestUri, $response.Headers['Location'])
            if ($next.Scheme -ne 'https' -or $next.Host -notin 'github.com','release-assets.githubusercontent.com','objects.githubusercontent.com' -or $next.UserInfo -or !$next.IsDefaultPort) {
                throw 'Untrusted release download redirect.'
            }
            $response.Dispose()
            $response = $null
            $request = [Net.HttpWebRequest]::Create($next)
            $request.AllowAutoRedirect = $false
            $request.Timeout = 15000
            $request.ReadWriteTimeout = 15000
            $request.UserAgent = 'FlitBrowserUpdate'
        }
        if ($response.ContentLength -gt $limit) { throw 'Release asset is too large.' }
        $inputStream = $response.GetResponseStream()
        try {
            $outputStream = [IO.File]::Open($destination, 'CreateNew', 'Write', 'None')
            try {
                $buffer = New-Object byte[] 65536
                $total = [long]0
                $clock = [Diagnostics.Stopwatch]::StartNew()
                while (($count = $inputStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $total += $count
                    if ($total -gt $limit -or $clock.Elapsed.TotalSeconds -gt 300) {
                        throw 'Release download exceeded its size or time limit.'
                    }
                    $outputStream.Write($buffer, 0, $count)
                }
                if ($total -eq 0) { throw 'Empty release asset.' }
            } finally { $outputStream.Dispose() }
        } finally { $inputStream.Dispose() }
    } finally { if ($response) { $response.Dispose() } }
}

function Read-FlitManifest([string]$manifest, [string]$signature, [string]$modulus) {
    if ((Get-Item -LiteralPath $manifest).Length -notin (1..4096) -or
        (Get-Item -LiteralPath $signature).Length -notin 256,384,512) {
        throw 'Invalid update signature or manifest size.'
    }
    $bytes = [IO.File]::ReadAllBytes($manifest)
    $parameters = [Security.Cryptography.RSAParameters]::new()
    $parameters.Modulus = [Convert]::FromBase64String($modulus)
    $parameters.Exponent = [byte[]]@(1,0,1)
    if ($parameters.Modulus.Length -notin 256,384,512) { throw 'Invalid public update key.' }
    $rsa = [Security.Cryptography.RSACryptoServiceProvider]::new()
    try {
        $rsa.PersistKeyInCsp = $false
        $rsa.ImportParameters($parameters)
        if (!$rsa.VerifyData($bytes, 'SHA256', [IO.File]::ReadAllBytes($signature))) {
            throw 'Update signature did not verify.'
        }
    } finally { $rsa.Dispose() }
    $feed = [Text.UTF8Encoding]::new($false,$true).GetString($bytes) | ConvertFrom-Json
    $version = Read-FlitVersion $feed.version
    if ([string]$feed.sha256 -notmatch '^[0-9a-fA-F]{64}$') { throw 'Invalid update SHA-256.' }
    return @{ version = $version; sha256 = [string]$feed.sha256 }
}

function Confirm-FlitAsset([string]$file, [hashtable]$feed) {
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ine $feed.sha256) {
        throw 'Installer SHA-256 did not verify.'
    }
    $product = [Diagnostics.FileVersionInfo]::GetVersionInfo($file).ProductVersion.Trim()
    if ((Read-FlitVersion $product) -ne $feed.version) { throw 'Installer version does not match the signed feed.' }
}

function Invoke-FlitUpdate {
    $mode = $env:FLIT_UPDATER_MODE
    $repo = $env:FLIT_UPDATER_REPO
    if ($mode -notin 'Check','Manual','Apply' -or $repo -notmatch '^[a-zA-Z0-9][a-zA-Z0-9-]{0,38}/[a-zA-Z0-9_.-]{1,100}$' -or $repo.Split('/')[-1] -in '.','..') { throw 'Invalid update channel.' }
    $modulus = $env:FLIT_UPDATER_PUBLIC_KEY
    $current = Read-FlitVersion $env:FLIT_UPDATER_CURRENT_VERSION
    $exe = [IO.Path]::GetFullPath($env:FLIT_UPDATER_EXE)
    $cache = [IO.Path]::GetFullPath($env:FLIT_UPDATER_CACHE)
    $expected = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Flit Browser\Updates'))
    if ($cache -ine $expected -or ([IO.File]::GetAttributes($cache) -band [IO.FileAttributes]::ReparsePoint)) { throw 'Invalid update cache path.' }
    Confirm-FlitInstall $exe $current
    $lock = $null
    $temporary = $null
    $status = Join-Path $cache 'status.txt'
    try {
        try { $lock = [IO.File]::Open((Join-Path $cache 'update.lock'), 'OpenOrCreate', 'ReadWrite', 'None') }
        catch [IO.IOException] { return }
        $last = Join-Path $cache 'last-check.txt'
        $ready = Join-Path $cache 'ready.txt'
        $manifest = Join-Path $cache 'flit-update.json'
        $signature = Join-Path $cache 'flit-update.json.sig'
        $setup = Join-Path $cache 'FlitSetup-x64.exe'
        if ($mode -eq 'Apply') {
            if (![IO.File]::Exists($ready)) { return }
            try {
                $feed = Read-FlitManifest $manifest $signature $modulus
                if ($feed.version -le $current) { throw 'A newer version is not staged.' }
                Confirm-FlitAsset $setup $feed
            } catch {
                [IO.File]::Delete($ready)
                throw
            }
            $parentId = [int]$env:FLIT_UPDATER_PARENT_PID
            $parent = Get-Process -Id $parentId -ErrorAction SilentlyContinue
            if ($parent) {
                try {
                    if ($parent.Path -ieq $exe -and !$parent.WaitForExit(45000)) {
                        throw 'Flit is still open. The update remains staged.'
                    }
                } finally { $parent.Dispose() }
            }
            $running = @(Get-Process -Name Flit -ErrorAction SilentlyContinue | Where-Object { $_.Path -ieq $exe })
            if ($running.Count -gt 0) { throw 'Flit is still open. The update remains staged.' }
            Confirm-FlitInstall $exe $current
            Confirm-FlitAsset $setup $feed
            $installDir = [IO.Path]::GetDirectoryName($exe)
            $process = Start-Process -FilePath $setup -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/SP-','/NORESTART','/NOCLOSEAPPLICATIONS','/NORESTARTAPPLICATIONS','/RESTARTEXITCODE=3010',('/DIR="' + $installDir + '"')) -Wait -PassThru -WindowStyle Hidden
            if ($process.ExitCode -notin 0,3010) { throw ('Installer failed with exit code {0}.' -f $process.ExitCode) }
            Confirm-FlitInstall $exe $feed.version
            [IO.File]::Delete($ready)
            Write-FlitText $status ('Installed {0}.' -f $feed.version)
            foreach ($file in @($manifest,$signature,$setup)) { [IO.File]::Delete($file) }
            return
        }
        if ($mode -eq 'Check' -and [IO.File]::Exists($last) -and [IO.File]::GetLastWriteTimeUtc($last) -gt [DateTime]::UtcNow.AddDays(-1)) { return }
        if ([IO.File]::Exists($ready)) { return }
        Write-FlitText $last ([DateTime]::UtcNow.ToString('o'))
        $base = "https://github.com/$repo/releases"
        $temporary = Join-Path $cache ('incoming-' + [guid]::NewGuid().ToString('N'))
        [IO.Directory]::CreateDirectory($temporary) | Out-Null
        $m = Join-Path $temporary 'flit-update.json'
        $s = Join-Path $temporary 'flit-update.json.sig'
        $x = Join-Path $temporary 'FlitSetup-x64.exe'
        Get-FlitAsset "$base/latest/download/flit-update.json" $m 4096
        Get-FlitAsset "$base/latest/download/flit-update.json.sig" $s 512
        $feed = Read-FlitManifest $m $s $modulus
        if ($feed.version -le $current) { Write-FlitText $status 'Flit is up to date.'; return }
        Get-FlitAsset "$base/download/v$($feed.version)/FlitSetup-x64.exe" $x 536870912
        Confirm-FlitAsset $x $feed
        foreach ($pair in @(@($m,$manifest),@($s,$signature),@($x,$setup))) {
            [IO.File]::Delete($pair[1])
            [IO.File]::Move($pair[0],$pair[1])
        }
        Write-FlitText $ready $feed.version.ToString()
        Write-FlitText $status ('Verified update {0} is ready.' -f $feed.version)
    } catch {
        Write-FlitText $status ($_.Exception.Message -replace '[\r\n]+',' ')
        throw
    } finally {
        if ($temporary) {
            foreach ($name in @('flit-update.json','flit-update.json.sig','FlitSetup-x64.exe')) { [IO.File]::Delete((Join-Path $temporary $name)) }
            if ([IO.Directory]::Exists($temporary)) { [IO.Directory]::Delete($temporary) }
        }
        if ($lock) { $lock.Dispose() }
    }
}

Invoke-FlitUpdate
