use std::ffi::OsStr;

use cef::{CefStringList, CommandLine, ImplCommandLine};
use url::Url;

/// Validate a literal web URL without rewriting its escapes or Unicode.
pub(crate) fn literal_http_url(input: &str) -> Option<&str> {
    let (scheme, tail) = input.split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || input
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\\'))
    {
        return None;
    }
    // The URL parser can repair an empty authority or discard empty userinfo.
    let authority = tail.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let parsed = Url::parse(input).ok()?;
    (parsed.host().is_some() && parsed.username().is_empty() && parsed.password().is_none())
        .then_some(input)
}

/// Read one positional URL from arguments after the executable name.
/// Leading Flit debug flags are left to the caller; `--` ends flag parsing.
pub(crate) fn from_args(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Option<String> {
    let mut args = args.into_iter().skip_while(|arg| {
        arg.as_ref()
            .to_str()
            .is_some_and(|arg| arg.starts_with("--flit-profile=") || arg.starts_with("--flit-url="))
    });
    let mut argument = args.next()?;
    if argument.as_ref() == OsStr::new("--") {
        argument = args.next()?;
    }
    if args.next().is_some() {
        return None;
    }
    literal_http_url(argument.as_ref().to_str()?).map(str::to_owned)
}

/// Read one positional URL from the read-only CEF relaunch command line.
pub(crate) fn from_cef_command_line(command_line: &CommandLine) -> Option<String> {
    let mut args = CefStringList::new();
    command_line.arguments(Some(&mut args));
    let mut args = args.into_iter();
    let argument = args.next()?;
    if args.next().is_some() {
        return None;
    }
    literal_http_url(&argument)?;
    Some(argument)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};

    const URL: &str = "HTTPS://예시.test/경로/🦊?keep=%2f%25&email=a@b#한글";

    #[test]
    fn literal_http_url_preserves_the_argument() {
        for input in [URL, "http://[::1]:8080/a"] {
            assert_eq!(literal_http_url(input), Some(input));
        }
    }

    #[test]
    fn literal_http_url_rejects_non_web_or_ambiguous_input() {
        for input in [
            r"C:\Users\test\page.html",
            r"\\server\share\page.html",
            "file:///C:/page.html",
            "chrome://settings/",
            "about:blank",
            "javascript:alert(1)",
            "example.test",
            "search terms",
            "--https://example.test/",
            "/https://example.test/",
            "--flit-url=https://example.test/",
            " https://example.test/",
            "https://example.test/a b",
            "https://example.test/a\n",
            "https://example.test/a\"b",
            r"https://example.test/a\b",
            "https:///example.test/",
            "https:example.test/",
            "https://",
            "https://example.test:65536/",
            "https://user:password@example.test/",
            "https://@example.test/",
            "https://%75ser@example.test/",
        ] {
            assert_eq!(literal_http_url(input), None, "{input:?}");
        }
    }

    #[test]
    fn accepts_exactly_one_positional_url_after_leading_debug_flags() {
        for args in [
            &[URL][..],
            &["--", URL],
            &[r"--flit-profile=C:\Scratch Flit", "--", URL],
            &["--flit-url=about:blank", URL],
        ] {
            assert_eq!(from_args(args).as_deref(), Some(URL), "{args:?}");
        }
        for args in [
            &[][..],
            &["--flit-url=about:blank"],
            &["--", "--flit-url=https://example.test/"],
            &["--", "--flit-profile=C:/other", URL],
            &["--unknown", URL],
            &["--", "--", URL],
            &["page.html", URL],
            &[URL, "https://other.test/"],
            &["--", URL, "--flit-profile=C:/other"],
        ] {
            assert_eq!(from_args(args), None, "{args:?}");
        }
        assert_eq!(from_args([OsString::from_wide(&[0xD800])]), None);
    }
}
