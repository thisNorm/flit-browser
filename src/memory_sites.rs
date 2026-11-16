use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

use url::{Host, Url};

const FILE_NAME: &str = "memory-sites.json";
const MAX_SITES: usize = 128;
const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_HOST_BYTES: usize = 253;
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

/// Per-profile exceptions keyed by the exact canonical HTTP(S) host.
#[derive(Default)]
pub(crate) struct SiteExceptions {
    sites: BTreeSet<String>,
}

impl SiteExceptions {
    pub(crate) fn load(profile: &Path) -> io::Result<Self> {
        let file = match File::open(profile.join(FILE_NAME)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Site exceptions file is too large",
            ));
        }
        let entries: Vec<String> = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if entries.len() > MAX_SITES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Too many site exceptions",
            ));
        }
        let mut sites = BTreeSet::new();
        for site in entries {
            if canonical_site(&site).as_deref() != Some(site.as_str()) || !sites.insert(site) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid or duplicate site exception",
                ));
            }
        }
        Ok(Self { sites })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    /// Look up a canonical key, as returned by [host].
    pub(crate) fn contains(&self, site: &str) -> bool {
        self.sites.contains(site)
    }

    /// Persist the toggle before updating memory. True means the site is now exempt.
    pub(crate) fn toggle(&mut self, profile: &Path, site: &str) -> io::Result<bool> {
        let site = canonical_site(site)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid site host"))?;
        let mut next = self.sites.clone();
        let added = !next.remove(&site);
        if added {
            if next.len() >= MAX_SITES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Too many site exceptions",
                ));
            }
            next.insert(site);
        }
        write_atomic(profile, &serde_json::to_vec(&next)?)?;
        self.sites = next;
        Ok(added)
    }
}

/// Extract the canonical host of a literal HTTP(S) URL, ignoring its port.
pub(crate) fn host(input: &str) -> Option<String> {
    let (scheme, rest) = input.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    // Reject even empty userinfo and malformed authorities the parser could repair.
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty()
        || authority.contains('@')
        || input
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\\'))
    {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if matches!(url.host()?, Host::Domain(domain) if !valid_domain(domain)) {
        return None;
    }
    url.host_str().map(str::to_owned)
}

fn canonical_site(input: &str) -> Option<String> {
    if input.len() > 4 * MAX_HOST_BYTES
        || input.contains(['%', '*', '@', '/', '?', '#', '\\'])
        || input.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    match Host::parse(input).ok()? {
        Host::Domain(domain) if valid_domain(&domain) => Some(domain),
        Host::Domain(_) => None,
        address => Some(address.to_string()),
    }
}

fn valid_domain(domain: &str) -> bool {
    let bare = domain.strip_suffix('.').unwrap_or(domain);
    bare.len() <= MAX_HOST_BYTES
        && bare.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_')
                })
        })
}

fn write_atomic(profile: &Path, bytes: &[u8]) -> io::Result<()> {
    let (temp, mut file) = loop {
        let number = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = profile.join(format!(".memory-sites-{}-{number}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temp, profile.join(FILE_NAME)));
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            loop {
                let number = NEXT.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("flit-memory-sites-{}-{number}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create scratch profile: {error}"),
                }
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join(FILE_NAME));
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn round_trip_adds_and_removes_only_the_exact_host() {
        let profile = Scratch::new();
        let other_profile = Scratch::new();
        let mut sites = SiteExceptions::load(&profile.0).unwrap();
        assert!(sites.is_empty());
        assert!(SiteExceptions::default().is_empty());

        let key = host("HTTPS://BÜCHER.Example:8443/a?email=a@b").unwrap();
        assert_eq!(key, "xn--bcher-kva.example");
        assert!(sites.toggle(&profile.0, &key).unwrap());
        assert!(sites.toggle(&profile.0, "LOCALHOST").unwrap());
        assert!(!sites.is_empty());
        assert!(SiteExceptions::load(&other_profile.0).unwrap().is_empty());

        let mut sites = SiteExceptions::load(&profile.0).unwrap();
        assert!(sites.contains(&key));
        assert!(sites.contains("localhost"));
        assert!(!sites.contains("sub.xn--bcher-kva.example"));
        assert!(!sites.contains("example"));
        assert!(!sites.toggle(&profile.0, "bücher.example").unwrap());
        assert!(sites.contains("localhost"));
        assert!(!SiteExceptions::load(&profile.0).unwrap().contains(&key));

        assert!(!sites.toggle(&profile.0, "localhost").unwrap());
        assert!(SiteExceptions::load(&profile.0).unwrap().is_empty());
        assert_eq!(fs::read(profile.0.join(FILE_NAME)).unwrap(), b"[]");
    }

    #[test]
    fn extracts_only_literal_http_hosts() {
        assert_eq!(host("http://127.1:8000/").as_deref(), Some("127.0.0.1"));
        assert_eq!(
            host("https://[2001:0DB8::1]:444/").as_deref(),
            Some("[2001:db8::1]")
        );
        assert_eq!(
            host("https://Example.com./").as_deref(),
            Some("example.com.")
        );
        for input in [
            "example.com",
            "chrome://example.com/",
            "https:///example.com/",
            "https://user:password@example.com/",
            "https://@example.com/",
            "https://example.com:65536/",
            "https://*.example.com/",
            "https://%2A.example.com/",
            "https://＊.example.com/",
            "https://example..com/",
            "https://example.com\\@elsewhere.test/",
        ] {
            assert_eq!(host(input), None, "{input:?}");
        }
    }

    #[test]
    fn rejects_invalid_or_oversized_files() {
        let profile = Scratch::new();
        let path = profile.0.join(FILE_NAME);
        for bytes in [
            br#"["example.com""#.as_slice(),
            br#"{"sites":[]}"#,
            br#"["https://example.com"]"#,
            br#"["user@example.com"]"#,
            br#"["*.example.com"]"#,
            br#"["Example.com"]"#,
            br#"["example.com","example.com"]"#,
            b"[\xff]",
        ] {
            fs::write(&path, bytes).unwrap();
            assert_eq!(
                SiteExceptions::load(&profile.0).err().unwrap().kind(),
                io::ErrorKind::InvalidData,
                "{bytes:?}"
            );
        }

        let mut bytes = vec![b' '; MAX_FILE_BYTES - 2];
        bytes.extend(b"[]");
        fs::write(&path, &bytes).unwrap();
        assert!(SiteExceptions::load(&profile.0).unwrap().is_empty());
        bytes.push(b' ');
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            SiteExceptions::load(&profile.0).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn enforces_capacity_without_preventing_removal() {
        let profile = Scratch::new();
        let path = profile.0.join(FILE_NAME);
        let names: Vec<_> = (0..=MAX_SITES).map(|i| format!("site-{i}.test")).collect();
        fs::write(&path, serde_json::to_vec(&names).unwrap()).unwrap();
        assert_eq!(
            SiteExceptions::load(&profile.0).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );

        let original = serde_json::to_vec(&names[..MAX_SITES]).unwrap();
        fs::write(&path, &original).unwrap();
        let mut sites = SiteExceptions::load(&profile.0).unwrap();
        assert!(sites.toggle(&profile.0, &names[MAX_SITES]).is_err());
        assert!(!sites.contains(&names[MAX_SITES]));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!sites.toggle(&profile.0, &names[0]).unwrap());
        assert!(sites.toggle(&profile.0, &names[MAX_SITES]).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn failed_replace_preserves_file_and_memory() {
        use std::os::windows::fs::OpenOptionsExt;

        let profile = Scratch::new();
        let path = profile.0.join(FILE_NAME);
        let mut sites = SiteExceptions::default();
        assert!(sites.toggle(&profile.0, "example.com").unwrap());
        let original = fs::read(&path).unwrap();

        let locked = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(sites.toggle(&profile.0, "example.com").is_err());
        assert!(sites.contains("example.com"));
        drop(locked);

        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&profile.0).unwrap().count(), 1);
    }
}
