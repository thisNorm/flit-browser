use std::ptr::{NonNull, null, null_mut};
use std::sync::atomic::{Ordering, compiler_fence};

use url::Url;
use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError, LocalFree};
use windows_sys::Win32::Security::Credentials::{
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_MAX_GENERIC_TARGET_NAME_LENGTH, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

const TARGET_PREFIX: &str = "FlitBrowser/AgentVault/v1/";
const MAX_USERNAME_BYTES: usize = 256;
const MAX_PASSWORD_BYTES: usize = 512;
const INVALID_RECORD: &str = "Flit Vault entry is invalid";

pub(crate) struct Credential {
    pub(crate) username: String,
    pub(crate) password: String,
}

impl Drop for Credential {
    fn drop(&mut self) {
        for value in [&mut self.username, &mut self.password] {
            // SAFETY: The allocation is writable up to capacity, and zeroes leave valid UTF-8.
            unsafe {
                let bytes = value.as_mut_vec();
                wipe(bytes.as_mut_ptr(), bytes.capacity());
            }
        }
    }
}

pub(crate) fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }

    // Inspect the raw authority before URL parsing can normalize away an escape or empty userinfo.
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty()
        || authority
            .chars()
            .any(|c| matches!(c, '@' | '%' | '\\') || c.is_control() || c.is_whitespace())
    {
        return None;
    }

    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https"
        || !parsed.has_host()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

pub(crate) fn has(origin: &str) -> Result<bool, &'static str> {
    // CredReadW returns the whole record; its blob stays encrypted for this check.
    Ok(StoredCredential::read(&target_name(origin)?)?.is_some())
}

pub(crate) fn load(origin: &str) -> Result<Option<Credential>, &'static str> {
    let mut target = target_name(origin)?;
    let Some(record) = StoredCredential::read(&target)? else {
        return Ok(None);
    };
    // SAFETY: A successful CredReadW owns this record until StoredCredential is dropped.
    let record_data = unsafe { record.0.as_ref() };
    if record_data.CredentialBlob.is_null()
        || !(1..=CRED_MAX_CREDENTIAL_BLOB_SIZE).contains(&record_data.CredentialBlobSize)
    {
        return Err(INVALID_RECORD);
    }
    let encrypted = CRYPT_INTEGER_BLOB {
        cbData: record_data.CredentialBlobSize,
        pbData: record_data.CredentialBlob,
    };
    let entropy = target_entropy(&mut target);
    let mut plaintext = CRYPT_INTEGER_BLOB::default();
    // SAFETY: Both input blobs reference live buffers, and the output is writable.
    if unsafe {
        CryptUnprotectData(
            &encrypted,
            null_mut(),
            &entropy,
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut plaintext,
        )
    } == 0
    {
        return Err("Could not decrypt Flit Vault entry");
    }
    let plaintext = DpapiBlob(plaintext);
    let size = plaintext.0.cbData as usize;
    if plaintext.0.pbData.is_null()
        || !(2..=2 + MAX_USERNAME_BYTES + MAX_PASSWORD_BYTES).contains(&size)
    {
        return Err(INVALID_RECORD);
    }
    // SAFETY: DPAPI returned this initialized buffer, which is kept alive by plaintext.
    let bytes = unsafe { std::slice::from_raw_parts(plaintext.0.pbData, size) };
    decode(bytes).map(Some)
}

pub(crate) fn save(origin: &str, username: &str, password: &str) -> Result<(), &'static str> {
    let mut target = target_name(origin)?;
    let mut plaintext = encode(username, password)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.0.len() as u32,
        pbData: plaintext.0.as_mut_ptr(),
    };
    let entropy = target_entropy(&mut target);
    let mut encrypted = CRYPT_INTEGER_BLOB::default();
    // SAFETY: Both input blobs reference live buffers, and the output is writable.
    let protected = unsafe {
        CryptProtectData(
            &input,
            null(),
            &entropy,
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut encrypted,
        )
    };
    drop(plaintext);
    if protected == 0 {
        return Err("Could not encrypt Flit Vault entry");
    }
    let encrypted = DpapiBlob(encrypted);
    if encrypted.0.pbData.is_null()
        || !(1..=CRED_MAX_CREDENTIAL_BLOB_SIZE).contains(&encrypted.0.cbData)
    {
        return Err("Encrypted Flit Vault entry has an invalid size");
    }

    let record = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: encrypted.0.cbData,
        CredentialBlob: encrypted.0.pbData,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: All record pointers remain valid until CredWriteW returns.
    if unsafe { CredWriteW(&record, 0) } == 0 {
        return Err("Could not save Flit Vault entry");
    }
    Ok(())
}

pub(crate) fn remove(origin: &str) -> Result<(), &'static str> {
    let target = target_name(origin)?;
    // SAFETY: The target is a live NUL-terminated UTF-16 buffer.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
        && unsafe { GetLastError() } != ERROR_NOT_FOUND
    {
        return Err("Could not remove Flit Vault entry");
    }
    Ok(())
}

fn target_name(canonical_origin: &str) -> Result<Vec<u16>, &'static str> {
    if canonical_origin.len() > CRED_MAX_GENERIC_TARGET_NAME_LENGTH as usize - TARGET_PREFIX.len()
        || origin(canonical_origin).as_deref() != Some(canonical_origin)
    {
        return Err("Flit Vault requires a canonical HTTPS origin");
    }
    Ok(TARGET_PREFIX
        .encode_utf16()
        .chain(canonical_origin.encode_utf16())
        .chain(std::iter::once(0))
        .collect())
}

fn target_entropy(target: &mut [u16]) -> CRYPT_INTEGER_BLOB {
    // Bind the ciphertext to the full Flit key so moving a blob to another origin cannot work.
    CRYPT_INTEGER_BLOB {
        cbData: std::mem::size_of_val(target) as u32,
        pbData: target.as_mut_ptr().cast(),
    }
}

struct StoredCredential(NonNull<CREDENTIALW>);

impl StoredCredential {
    fn read(target: &[u16]) -> Result<Option<Self>, &'static str> {
        let mut record = null_mut();
        // SAFETY: The target is NUL-terminated UTF-16 and record is a writable output pointer.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut record) } == 0 {
            return match unsafe { GetLastError() } {
                ERROR_NOT_FOUND => Ok(None),
                _ => Err("Could not read Flit Vault entry"),
            };
        }
        NonNull::new(record)
            .map(Self)
            .map(Some)
            .ok_or(INVALID_RECORD)
    }
}

impl Drop for StoredCredential {
    fn drop(&mut self) {
        // SAFETY: This is the allocation returned by CredReadW.
        unsafe { CredFree(self.0.as_ptr().cast()) };
    }
}

struct DpapiBlob(CRYPT_INTEGER_BLOB);

impl Drop for DpapiBlob {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // SAFETY: DPAPI owns a writable allocation of cbData bytes and requires LocalFree.
            unsafe {
                wipe(self.0.pbData, self.0.cbData as usize);
                LocalFree(self.0.pbData.cast());
            }
        }
    }
}

struct SecretBytes(Vec<u8>);

impl Drop for SecretBytes {
    fn drop(&mut self) {
        // SAFETY: The Vec owns writable storage up to capacity.
        unsafe { wipe(self.0.as_mut_ptr(), self.0.capacity()) };
    }
}

// Caller must supply exclusive, writable storage for len bytes, which may be uninitialized.
unsafe fn wipe(ptr: *mut u8, len: usize) {
    for offset in 0..len {
        // SAFETY: The caller guarantees that this offset lies in the allocation.
        unsafe { ptr.add(offset).write_volatile(0) };
    }
    compiler_fence(Ordering::SeqCst);
}

fn encode(username: &str, password: &str) -> Result<SecretBytes, &'static str> {
    if username.len() > MAX_USERNAME_BYTES || password.len() > MAX_PASSWORD_BYTES {
        return Err("Flit Vault limits usernames to 256 and passwords to 512 UTF-8 bytes");
    }
    let mut bytes = SecretBytes(Vec::with_capacity(2 + username.len() + password.len()));
    bytes
        .0
        .extend_from_slice(&(username.len() as u16).to_le_bytes());
    bytes.0.extend_from_slice(username.as_bytes());
    bytes.0.extend_from_slice(password.as_bytes());
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<Credential, &'static str> {
    let (len, payload) = bytes.split_first_chunk::<2>().ok_or(INVALID_RECORD)?;
    let (username, password) = payload
        .split_at_checked(u16::from_le_bytes(*len) as usize)
        .ok_or(INVALID_RECORD)?;
    if username.len() > MAX_USERNAME_BYTES || password.len() > MAX_PASSWORD_BYTES {
        return Err(INVALID_RECORD);
    }
    let username = std::str::from_utf8(username).map_err(|_| INVALID_RECORD)?;
    let password = std::str::from_utf8(password).map_err(|_| INVALID_RECORD)?;
    Ok(Credential {
        username: username.to_owned(),
        password: password.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_canonical_https_origin_from_a_url() {
        for (url, expected) in [
            (
                "HTTPS://Example.Test:443/a%2Fb\\c@d?q=%40#fragment",
                "https://example.test",
            ),
            (
                "https://BÜCHER.example:8443/login",
                "https://xn--bcher-kva.example:8443",
            ),
            ("https://[0:0:0:0:0:0:0:1]:4443/", "https://[::1]:4443"),
        ] {
            assert_eq!(origin(url).as_deref(), Some(expected));
        }
    }

    #[test]
    fn rejects_non_https_and_ambiguous_authorities() {
        for url in [
            "http://127.0.0.1:4443/",
            "blob:https://example.test/id",
            "//example.test/login",
            " https://example.test",
            "https:///example.test",
            "https://@example.test",
            "https://user:dummy@example.test",
            "https://%65xample.test",
            "https://example.test\\login",
            "https://exam\tple.test",
            "https://example.test ",
            "https://example.test\0",
        ] {
            assert!(origin(url).is_none(), "accepted {url:?}");
        }
    }

    #[test]
    fn lookup_key_requires_an_exact_canonical_origin() {
        let target = target_name("https://example.test:4443").unwrap();
        assert_eq!(
            target,
            "FlitBrowser/AgentVault/v1/https://example.test:4443\0"
                .encode_utf16()
                .collect::<Vec<_>>()
        );
        for key in [
            "https://EXAMPLE.test:4443",
            "https://example.test:443",
            "https://example.test/",
            "https://example.test/login",
            "https://example.test?",
            "https://example.test#",
            "FlitBrowser/AgentVault/v1/https://example.test",
        ] {
            assert!(target_name(key).is_err(), "accepted {key:?}");
        }
    }

    #[test]
    fn encoding_preserves_utf8_and_embedded_zero_bytes() {
        let bytes = encode("é", "가\0!").unwrap();
        assert_eq!(bytes.0, b"\x02\x00\xc3\xa9\xea\xb0\x80\x00!");
        let decoded = decode(&bytes.0).unwrap();
        assert_eq!(decoded.username, "é");
        assert_eq!(decoded.password, "가\0!");
    }

    #[test]
    fn enforces_utf8_byte_limits_on_both_sides() {
        let username = "é".repeat(128);
        let password = "é".repeat(256);
        let bytes = encode(&username, &password).unwrap();
        let decoded = decode(&bytes.0).unwrap();
        assert_eq!(decoded.username, username);
        assert_eq!(decoded.password, password);
        assert!(encode(&format!("{username}a"), &password).is_err());
        assert!(encode(&username, &format!("{password}a")).is_err());

        let oversized_username = [b"\x01\x01".as_slice(), &[b'u'; 257], b"p"].concat();
        let oversized_password = [b"\x01\x00u".as_slice(), &[b'p'; 513]].concat();
        assert!(decode(&oversized_username).is_err());
        assert!(decode(&oversized_password).is_err());
    }

    #[test]
    fn rejects_malformed_encoded_credentials() {
        for bytes in [
            b"\x00".as_slice(),
            b"\x02\x00u",
            b"\x01\x00\xffp",
            b"\x01\x00u\xff",
        ] {
            assert!(decode(bytes).is_err());
        }
    }
}
