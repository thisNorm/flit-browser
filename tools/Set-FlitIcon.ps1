# Produce a fresh Flit.exe for the unsigned preview. The CEF input is read-only.
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$BootstrapPath,
    [Parameter(Mandatory)]
    [string]$OutputPath,
    [string]$IconPath
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This script requires Windows.' }
if (!$IconPath) { $IconPath = Join-Path $PSScriptRoot '../assets/Flit.ico' }
$bootstrap = Get-Item -LiteralPath $BootstrapPath
$ico = Get-Item -LiteralPath $IconPath
$output = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputPath)
if ($bootstrap -isnot [IO.FileInfo] -or $bootstrap.Name -ine 'bootstrap.exe') {
    throw 'Pass the original CEF bootstrap.exe as -BootstrapPath.'
}
if ($ico -isnot [IO.FileInfo] -or $ico.Extension -ine '.ico' -or $ico.Length -gt 4MB) {
    throw 'Pass a Windows .ico file up to 4 MiB.'
}
if ([IO.Path]::GetFileName($output) -ine 'Flit.exe' -or (Test-Path -LiteralPath $output)) {
    throw '-OutputPath must be a new file named Flit.exe; existing files are never overwritten.'
}
$parent = [IO.Path]::GetDirectoryName($output)
if (![IO.Directory]::Exists($parent)) { throw "Create the output directory first: $parent" }

if (-not ('FlitPreview.IconStamp' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;

namespace FlitPreview {
    public static class IconStamp {
        const int RtIcon = 3, RtGroupIcon = 14, GroupId = 32512;
        const uint ResourceEnumLn = 1, LoadAsResource = 0x20 | 0x40;

        delegate bool NameCallback(IntPtr module, IntPtr type, IntPtr name, IntPtr param);
        delegate bool LanguageCallback(IntPtr module, IntPtr type, IntPtr name, ushort language, IntPtr param);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
        static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool FreeLibrary(IntPtr module);
        [DllImport("kernel32.dll", ExactSpelling = true, SetLastError = true)]
        static extern bool EnumResourceNamesExW(IntPtr module, IntPtr type, NameCallback callback,
            IntPtr param, uint flags, ushort language);
        [DllImport("kernel32.dll", ExactSpelling = true, SetLastError = true)]
        static extern bool EnumResourceLanguagesExW(IntPtr module, IntPtr type, IntPtr name,
            LanguageCallback callback, IntPtr param, uint flags, ushort language);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
        static extern IntPtr BeginUpdateResourceW(string path, bool deleteExisting);
        [DllImport("kernel32.dll", ExactSpelling = true, SetLastError = true)]
        static extern bool UpdateResourceW(IntPtr update, IntPtr type, IntPtr name, ushort language,
            byte[] data, uint size);
        [DllImport("kernel32.dll", ExactSpelling = true, SetLastError = true)]
        static extern bool EndUpdateResourceW(IntPtr update, bool discard);

        static Win32Exception Failed(string call) {
            int error = Marshal.GetLastWin32Error();
            return new Win32Exception(error, call + ": " + new Win32Exception(error).Message);
        }

        static List<int> Names(IntPtr module, int type) {
            var names = new List<int>();
            if (!EnumResourceNamesExW(module, (IntPtr)type, (m, t, name, p) => {
                long value = name.ToInt64();
                names.Add(value >= 0 && value <= UInt16.MaxValue ? (int)value : -1);
                return true;
            }, IntPtr.Zero, ResourceEnumLn, 0))
                throw Failed("EnumResourceNamesExW");
            return names;
        }

        static void Put(IntPtr update, int type, int name, ushort language, byte[] data) {
            if (!UpdateResourceW(update, (IntPtr)type, (IntPtr)name, language, data, (uint)data.Length))
                throw Failed("UpdateResourceW");
        }

        public static int Stamp(string path, byte[] ico) {
            if (ico.Length < 6 || ico.Length > 4 * 1024 * 1024 ||
                BitConverter.ToUInt16(ico, 0) != 0 || BitConverter.ToUInt16(ico, 2) != 1)
                throw new InvalidDataException("Invalid ICO header.");
            int count = BitConverter.ToUInt16(ico, 4);
            int directoryEnd = 6 + 16 * count;
            if (count < 1 || count > 64 || ico.Length < directoryEnd)
                throw new InvalidDataException("Invalid ICO frame count.");

            var frames = new byte[count][];
            var group = new byte[6 + 14 * count];
            Buffer.BlockCopy(ico, 0, group, 0, 6);
            for (int i = 0; i < count; i++) {
                int entry = 6 + 16 * i;
                uint size = BitConverter.ToUInt32(ico, entry + 8);
                uint offset = BitConverter.ToUInt32(ico, entry + 12);
                if (ico[entry] != ico[entry + 1] || ico[entry + 3] != 0 ||
                    size == 0 || offset < directoryEnd || (long)offset + size > ico.Length)
                    throw new InvalidDataException("Invalid ICO frame " + i + ".");
                frames[i] = new byte[size];
                Buffer.BlockCopy(ico, (int)offset, frames[i], 0, (int)size);
                Buffer.BlockCopy(ico, entry, group, 6 + 14 * i, 12);
            }

            IntPtr module = LoadLibraryExW(path, IntPtr.Zero, LoadAsResource);
            if (module == IntPtr.Zero) throw Failed("LoadLibraryExW");
            var languages = new List<ushort>();
            HashSet<int> used;
            try {
                if (Names(module, RtGroupIcon)[0] != GroupId)
                    throw new InvalidDataException("Expected the first icon group to be ID 32512.");
                if (!EnumResourceLanguagesExW(module, (IntPtr)RtGroupIcon, (IntPtr)GroupId,
                    (m, t, n, lang, p) => { languages.Add(lang); return true; },
                    IntPtr.Zero, ResourceEnumLn, 0) || languages.Count == 0)
                    throw Failed("EnumResourceLanguagesExW");
                used = new HashSet<int>(Names(module, RtIcon));
            } finally {
                FreeLibrary(module);
            }

            // Allocate unused IDs; even old frames could be referenced by other icon groups.
            var ids = new int[count];
            int next = 1;
            for (int i = 0; i < count; i++) {
                while (used.Contains(next)) next++;
                if (next > UInt16.MaxValue) throw new InvalidDataException("No free RT_ICON IDs.");
                ids[i] = next++;
                group[18 + 14 * i] = (byte)ids[i];
                group[19 + 14 * i] = (byte)(ids[i] >> 8);
            }

            IntPtr update = BeginUpdateResourceW(path, false);
            if (update == IntPtr.Zero) throw Failed("BeginUpdateResourceW");
            try {
                foreach (ushort language in languages) {
                    for (int i = 0; i < count; i++)
                        Put(update, RtIcon, ids[i], language, frames[i]);
                    Put(update, RtGroupIcon, GroupId, language, group);
                }
                bool committed = EndUpdateResourceW(update, false);
                update = IntPtr.Zero;
                if (!committed) throw Failed("EndUpdateResourceW");
            } finally {
                if (update != IntPtr.Zero) EndUpdateResourceW(update, true);
            }
            return count;
        }
    }
}
'@
}

# Pin the inspected official unsigned Windows x64 bootstraps.
$expectedHashes = @(
    'BDCF5C7A978F810E626E6CB923F1AA06477F0114CBB74515662F2157386F7472', # CEF 156.0.26
    '4A559CE8B73B0D482752E1343601FB1BF2D17EC1CD6D2BC6B7BE411D1F41207F'  # CEF 158.0.3 beta
)
$stage = Join-Path $parent ('.flit-icon-' + [guid]::NewGuid().ToString('N') + '.exe')
$created = $false
try {
    $source = [IO.File]::OpenRead($bootstrap.FullName)
    try {
        if ((Get-AuthenticodeSignature -LiteralPath $bootstrap.FullName).Status -ne 'NotSigned') {
            throw 'Signed or unverifiable bootstraps cannot be stamped for this preview.'
        }
        if ((Get-FileHash -InputStream $source -Algorithm SHA256).Hash -notin $expectedHashes) {
            throw 'Unexpected bootstrap.exe. Use a verified CEF Windows x64 bootstrap for this preview.'
        }
        $source.Position = 0
        $copy = [IO.File]::Open($stage, [IO.FileMode]::CreateNew)
        $created = $true
        try { $source.CopyTo($copy) } finally { $copy.Dispose() }
    } finally {
        $source.Dispose()
    }
    $count = [FlitPreview.IconStamp]::Stamp($stage, [IO.File]::ReadAllBytes($ico.FullName))
    [IO.File]::Move($stage, $output)
    Write-Verbose "Stamped $count icon frames into RT_GROUP_ICON 32512."
    Get-Item -LiteralPath $output
} finally {
    if ($created) { [IO.File]::Delete($stage) }
}
