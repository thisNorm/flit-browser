use url::Url;

pub(crate) const HOME: &str = "about:blank";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_search_from_network_and_local_development_urls() {
        assert_eq!(
            address(" example.com/docs ").unwrap(),
            "https://example.com/docs"
        );
        assert_eq!(address("localhost:8090").unwrap(), "http://localhost:8090/");
        assert_eq!(
            address("127.0.0.1:8080/a").unwrap(),
            "http://127.0.0.1:8080/a"
        );
        assert_eq!(address("[::1]:8080/a").unwrap(), "http://[::1]:8080/a");
        assert_eq!(
            address("HTTPS://EXAMPLE.COM").unwrap(),
            "https://example.com/"
        );
        assert_eq!(
            address("flit test & cache").unwrap(),
            "https://duckduckgo.com/?q=flit+test+%26+cache"
        );
    }

    #[test]
    fn rejects_nonweb_schemes_and_credentials() {
        for input in [
            "file:///C:/secret",
            "javascript:alert(1)",
            "data:text/html,hello",
            "chrome://crash/",
            "https://user:password@example.com/",
            "http://",
            " ",
        ] {
            assert!(address(input).is_err(), "{input}");
        }
        assert_eq!(address("about:blank").unwrap(), HOME);
        assert_eq!(address("chrome://settings/").unwrap(), "chrome://settings/");
        assert!(address("chrome://user@settings/").is_err());
    }
}

pub(crate) fn address(input: &str) -> Result<String, &'static str> {
    let input = input.trim();
    if input == HOME {
        return Ok(HOME.into());
    }
    if input.is_empty() || input.chars().any(char::is_control) {
        return Err("Enter an address or search terms.");
    }
    let authority = input.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(authority, |(host, _)| host);
    let local = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
        || host == "[::1]";
    let has_space = input.chars().any(char::is_whitespace);
    let domain = host.contains('.') && !host.contains(['@', '\\', ' ']);
    let explicit_scheme = input.split_once(':').is_some_and(|(s, _)| {
        s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    });
    if !has_space && (local || domain) && !input.contains("://") {
        let protocol = if local { "http" } else { "https" };
        return web_url(&format!("{protocol}://{input}"));
    }
    if explicit_scheme {
        return web_url(input);
    }
    let mut search = Url::parse("https://duckduckgo.com/").expect("static search URL");
    search.query_pairs_mut().append_pair("q", input);
    Ok(search.into())
}

fn web_url(input: &str) -> Result<String, &'static str> {
    let url = Url::parse(input).map_err(|_| "The address is invalid.")?;
    let builtin = url.scheme() == "chrome"
        && matches!(
            url.host_str(),
            Some(
                "settings"
                    | "history"
                    | "bookmarks"
                    | "downloads"
                    | "extensions"
                    | "password-manager"
                    | "version"
            )
        );
    if (!matches!(url.scheme(), "http" | "https") && !builtin)
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Enter an HTTP or HTTPS address without credentials.");
    }
    Ok(url.into())
}
