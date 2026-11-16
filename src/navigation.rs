use serde::{Deserialize, Serialize};
use url::Url;

pub(crate) const HOME: &str = "about:blank";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SearchEngine {
    Google,
    Naver,
    Bing,
    #[default]
    #[serde(rename = "duckduckgo", other)]
    DuckDuckGo,
}

impl SearchEngine {
    pub(crate) const ALL: [(Self, &'static str); 4] = [
        (Self::DuckDuckGo, "DuckDuckGo"),
        (Self::Google, "Google"),
        (Self::Naver, "Naver"),
        (Self::Bing, "Bing"),
    ];

    pub(crate) fn is_default(&self) -> bool {
        *self == Self::DuckDuckGo
    }

    fn search(self, keyword: &str) -> String {
        let (base, parameter) = match self {
            Self::DuckDuckGo => ("https://duckduckgo.com/", "q"),
            Self::Google => ("https://www.google.com/search", "q"),
            Self::Naver => ("https://search.naver.com/search.naver", "query"),
            Self::Bing => ("https://www.bing.com/search", "q"),
        };
        let mut url = Url::parse(base).expect("static search URL");
        url.query_pairs_mut().append_pair(parameter, keyword);
        url.into()
    }
}

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

    #[test]
    fn explicit_search_keeps_address_like_queries_as_search_terms() {
        let query = "https://example.test/한글?q=x+y&z=#끝";
        for (engine, host, path, field) in [
            (SearchEngine::DuckDuckGo, "duckduckgo.com", "/", "q"),
            (SearchEngine::Google, "www.google.com", "/search", "q"),
            (
                SearchEngine::Naver,
                "search.naver.com",
                "/search.naver",
                "query",
            ),
            (SearchEngine::Bing, "www.bing.com", "/search", "q"),
        ] {
            let target = address_bar(&format!(" ? {query} "), engine).unwrap();
            let parsed = Url::parse(&target).unwrap();
            assert_eq!(parsed.scheme(), "https");
            assert_eq!(parsed.host_str(), Some(host));
            assert_eq!(parsed.path(), path);
            assert_eq!(parsed.fragment(), None);
            assert_eq!(
                parsed.query_pairs().collect::<Vec<_>>(),
                [(field.into(), query.into())]
            );
            assert_eq!(
                address_bar("example.test", engine).unwrap(),
                "https://example.test/"
            );
            assert!(address_bar("?   ", engine).is_err());
            assert!(address_bar("? x\ny", engine).is_err());
        }
        assert_eq!(
            address_bar("브라우저 검색", SearchEngine::Naver).unwrap(),
            "https://search.naver.com/search.naver?query=%EB%B8%8C%EB%9D%BC%EC%9A%B0%EC%A0%80+%EA%B2%80%EC%83%89"
        );
    }
}

pub(crate) fn address(input: &str) -> Result<String, &'static str> {
    resolve(input, SearchEngine::default())
}

pub(crate) fn address_bar(input: &str, engine: SearchEngine) -> Result<String, &'static str> {
    if let Some(term) = input.trim().strip_prefix('?') {
        let term = term.trim();
        if term.is_empty() || term.chars().any(char::is_control) {
            return Err("Enter search terms.");
        }
        return Ok(engine.search(term));
    }
    resolve(input, engine)
}

fn resolve(input: &str, engine: SearchEngine) -> Result<String, &'static str> {
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
    Ok(engine.search(input))
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
