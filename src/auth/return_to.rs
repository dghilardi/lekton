//! Where to send a reader after signing in.
//!
//! Sign-in links carry the page that asked for them as `?next=`. The value
//! comes back from the browser (and, for OAuth, through the provider), so it is
//! only ever accepted as a same-origin path: never a scheme, a host, or a
//! protocol-relative `//host` that browsers would follow off-site.

/// Query parameter carrying the page to return to after signing in.
pub const NEXT_PARAM: &str = "next";

const MAX_RETURN_PATH_LEN: usize = 2048;

/// The same-origin path to return to, or `None` when `next` is absent or not a
/// plain local path. Sign-in and auth endpoints are refused so a login never
/// lands back on itself.
pub fn safe_return_path(next: Option<&str>) -> Option<String> {
    let next = next?.trim();
    let local = next.starts_with('/')
        && !next.starts_with("//")
        && !next.contains('\\')
        && next.len() <= MAX_RETURN_PATH_LEN
        && !next.chars().any(char::is_control);
    if !local {
        return None;
    }
    let path = next.split(['?', '#']).next().unwrap_or_default();
    let is_auth_path = path == "/login"
        || path.starts_with("/login/")
        || path.starts_with("/auth/")
        || path.starts_with("/api/");
    (!is_auth_path).then(|| next.to_string())
}

/// The sign-in page, returning to `return_to` afterwards when it is a local
/// page worth returning to (the home page needs no parameter).
pub fn login_href(base: &str, return_to: &str) -> String {
    match safe_return_path(Some(return_to)) {
        Some(path) if path != "/" => format!("{base}?{NEXT_PARAM}={}", encode_component(&path)),
        _ => base.to_string(),
    }
}

/// The sign-in page for the page the browser is on, for redirects made outside
/// the router (an expired session found by a background call). `None` when the
/// browser is already there, so a late call cannot drop its `next`.
#[cfg(feature = "hydrate")]
pub fn current_login_href() -> Option<String> {
    let location = web_sys::window()?.location();
    let pathname = location.pathname().ok()?;
    if pathname == "/login" {
        return None;
    }
    Some(login_href(
        "/login",
        &format!("{pathname}{}", location.search().ok()?),
    ))
}

/// Percent-encodes everything but RFC 3986 unreserved characters and `/`.
fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths_are_kept_with_their_query() {
        assert_eq!(
            safe_return_path(Some("/docs/handbook?pin=orders@1.0.0#faq")).as_deref(),
            Some("/docs/handbook?pin=orders@1.0.0#faq")
        );
        assert_eq!(safe_return_path(Some("/")).as_deref(), Some("/"));
    }

    #[test]
    fn anything_that_could_leave_the_site_is_refused() {
        for next in [
            "https://evil.example/",
            "//evil.example/",
            "/\\evil.example",
            "\\\\evil.example",
            "javascript:alert(1)",
            "docs/relative",
            "/docs/\nheader",
            "",
        ] {
            assert_eq!(safe_return_path(Some(next)), None, "{next:?}");
        }
        assert_eq!(safe_return_path(None), None);
        assert_eq!(
            safe_return_path(Some(&format!("/{}", "a".repeat(3000)))),
            None
        );
    }

    #[test]
    fn sign_in_and_auth_endpoints_are_refused() {
        for next in [
            "/login",
            "/login?next=/x",
            "/auth/callback",
            "/api/auth/logout",
        ] {
            assert_eq!(safe_return_path(Some(next)), None, "{next:?}");
        }
        assert!(safe_return_path(Some("/login-help")).is_some());
    }

    #[test]
    fn login_links_carry_the_encoded_page() {
        assert_eq!(
            login_href("/login", "/docs/a b?pin=x@1&y=2"),
            "/login?next=/docs/a%20b%3Fpin%3Dx%401%26y%3D2"
        );
        assert_eq!(login_href("/auth/login", "/"), "/auth/login");
        assert_eq!(login_href("/login", "//evil.example"), "/login");
    }
}
