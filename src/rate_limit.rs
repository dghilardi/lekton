use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request};
use axum_extra::extract::CookieJar;
use ipnet::IpNet;
use tower_governor::errors::GovernorError;
use tower_governor::key_extractor::KeyExtractor;

use crate::auth::extractor::ACCESS_TOKEN_COOKIE;
use crate::auth::token_service::TokenService;

const X_FORWARDED_FOR: &str = "x-forwarded-for";
const X_REAL_IP: &str = "x-real-ip";
const FORWARDED: &str = "forwarded";

#[derive(Debug, Clone)]
pub struct TrustedProxyIpKeyExtractor {
    trusted_proxies: Vec<IpNet>,
}

impl TrustedProxyIpKeyExtractor {
    pub fn from_config(raw: &str) -> Result<Self, String> {
        let trusted_proxies = raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(parse_proxy_net)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { trusted_proxies })
    }

    fn trusts(&self, peer: IpAddr) -> bool {
        self.trusted_proxies.iter().any(|net| net.contains(&peer))
    }
}

impl KeyExtractor for TrustedProxyIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let peer_ip = peer_ip(req).ok_or(GovernorError::UnableToExtractKey)?;

        if self.trusts(peer_ip) {
            Ok(forwarded_client_ip(req.headers()).unwrap_or(peer_ip))
        } else {
            Ok(peer_ip)
        }
    }
}

/// Rate-limit key that follows the authenticated user, falling back to the
/// client IP when there is none.
///
/// The IP-keyed limiter is the wrong shape for the endpoints that call an LLM:
/// behind a corporate proxy everyone shares one address, so a per-IP quota is
/// either loose enough to be useless or tight enough to punish the whole
/// office for one caller's loop. Keying on the user makes the quota personal.
///
/// Unauthenticated callers still fall back to the IP, and so do demo sessions —
/// demo mode is a development affordance, not a cost surface worth the extra
/// cookie handling.
#[derive(Clone)]
pub struct UserOrIpKeyExtractor {
    token_service: Arc<TokenService>,
    ip: TrustedProxyIpKeyExtractor,
}

impl UserOrIpKeyExtractor {
    pub fn new(token_service: Arc<TokenService>, ip: TrustedProxyIpKeyExtractor) -> Self {
        Self { token_service, ip }
    }

    fn user_id<T>(&self, req: &Request<T>) -> Option<String> {
        CookieJar::from_headers(req.headers())
            .get(ACCESS_TOKEN_COOKIE)
            .and_then(|cookie| {
                self.token_service
                    .validate_access_token(cookie.value())
                    .ok()
            })
            .map(|claims| claims.sub)
    }
}

impl KeyExtractor for UserOrIpKeyExtractor {
    /// Prefixed so a user id can never collide with an address.
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        match self.user_id(req) {
            Some(user_id) => Ok(format!("user:{user_id}")),
            None => Ok(format!("ip:{}", self.ip.extract(req)?)),
        }
    }
}

fn parse_proxy_net(entry: &str) -> Result<IpNet, String> {
    if entry.contains('/') {
        return entry
            .parse::<IpNet>()
            .map_err(|err| format!("invalid trusted proxy CIDR '{entry}': {err}"));
    }

    let ip = entry
        .parse::<IpAddr>()
        .map_err(|err| format!("invalid trusted proxy IP '{entry}': {err}"))?;
    let prefix_len = if ip.is_ipv4() { 32 } else { 128 };
    IpNet::new(ip, prefix_len).map_err(|err| format!("invalid trusted proxy IP '{entry}': {err}"))
}

fn peer_ip<T>(req: &Request<T>) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|addr| addr.ip())
        .or_else(|| req.extensions().get::<SocketAddr>().map(|addr| addr.ip()))
}

fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    forwarded_for_header(headers)
        .or_else(|| real_ip_header(headers))
        .or_else(|| forwarded_header(headers))
}

fn forwarded_for_header(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_FORWARDED_FOR)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').find_map(parse_ip_identifier))
}

fn real_ip_header(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(X_REAL_IP)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_ip_identifier)
}

fn forwarded_header(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all(FORWARDED)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .find_map(|entry| {
            entry.split(';').find_map(|part| {
                let (name, value) = part.split_once('=')?;
                if name.trim().eq_ignore_ascii_case("for") {
                    parse_ip_identifier(value)
                } else {
                    None
                }
            })
        })
}

fn parse_ip_identifier(value: &str) -> Option<IpAddr> {
    let value = value.trim().trim_matches('"');
    if value.is_empty() || value.eq_ignore_ascii_case("unknown") || value.starts_with('_') {
        return None;
    }

    if let Ok(ip) = value.parse::<IpAddr>() {
        return Some(ip);
    }

    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Some(addr.ip());
    }

    if let Some(stripped) = value.strip_prefix('[') {
        let end = stripped.find(']')?;
        return stripped[..end].parse::<IpAddr>().ok();
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::HeaderName;
    use axum::http::Request;

    fn request(peer: SocketAddr, headers: &[(&str, &str)]) -> Request<()> {
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(ConnectInfo(peer));
        for (name, value) in headers {
            req.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        req
    }

    fn user_or_ip_extractor() -> UserOrIpKeyExtractor {
        UserOrIpKeyExtractor::new(
            Arc::new(TokenService::new(
                "test-secret-at-least-32-bytes-long",
                900,
                7,
            )),
            TrustedProxyIpKeyExtractor::from_config("127.0.0.1").unwrap(),
        )
    }

    fn authenticated_user(user_id: &str) -> crate::auth::models::AuthenticatedUser {
        crate::auth::models::AuthenticatedUser {
            user_id: user_id.to_string(),
            email: "u@example.com".to_string(),
            name: None,
            is_admin: false,
        }
    }

    #[test]
    fn keys_on_the_user_when_a_valid_token_is_present() {
        let extractor = user_or_ip_extractor();
        let token = Arc::new(TokenService::new(
            "test-secret-at-least-32-bytes-long",
            900,
            7,
        ))
        .generate_access_token(&authenticated_user("u-42"))
        .expect("token");
        let req = request(
            "198.51.100.5:12345".parse().unwrap(),
            &[("cookie", &format!("{ACCESS_TOKEN_COOKIE}={token}"))],
        );

        assert_eq!(extractor.extract(&req).unwrap(), "user:u-42");
    }

    #[test]
    fn falls_back_to_the_ip_without_a_token() {
        let extractor = user_or_ip_extractor();
        let req = request("198.51.100.5:12345".parse().unwrap(), &[]);

        assert_eq!(extractor.extract(&req).unwrap(), "ip:198.51.100.5");
    }

    #[test]
    fn falls_back_to_the_ip_when_the_token_is_not_ours() {
        // A forged or expired cookie must not become a key of its own: it would
        // let a caller mint unlimited quotas by varying the cookie value.
        let extractor = user_or_ip_extractor();
        let req = request(
            "198.51.100.5:12345".parse().unwrap(),
            &[("cookie", &format!("{ACCESS_TOKEN_COOKIE}=not-a-jwt"))],
        );

        assert_eq!(extractor.extract(&req).unwrap(), "ip:198.51.100.5");
    }

    #[test]
    fn uses_forwarded_for_when_peer_is_trusted() {
        let extractor = TrustedProxyIpKeyExtractor::from_config("127.0.0.1").unwrap();
        let req = request(
            "127.0.0.1:12345".parse().unwrap(),
            &[("x-forwarded-for", "203.0.113.10, 127.0.0.1")],
        );

        assert_eq!(
            extractor.extract(&req).unwrap(),
            "203.0.113.10".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn ignores_spoofed_forwarded_for_when_peer_is_untrusted() {
        let extractor = TrustedProxyIpKeyExtractor::from_config("127.0.0.1").unwrap();
        let req = request(
            "198.51.100.5:12345".parse().unwrap(),
            &[("x-forwarded-for", "203.0.113.10")],
        );

        assert_eq!(
            extractor.extract(&req).unwrap(),
            "198.51.100.5".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn supports_trusted_proxy_cidr() {
        let extractor = TrustedProxyIpKeyExtractor::from_config("10.0.0.0/8").unwrap();
        let req = request(
            "10.12.0.4:12345".parse().unwrap(),
            &[("x-real-ip", "203.0.113.20")],
        );

        assert_eq!(
            extractor.extract(&req).unwrap(),
            "203.0.113.20".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn parses_forwarded_header() {
        let extractor = TrustedProxyIpKeyExtractor::from_config("127.0.0.1").unwrap();
        let req = request(
            "127.0.0.1:12345".parse().unwrap(),
            &[("forwarded", "for=\"[2001:db8::1]:443\";proto=https")],
        );

        assert_eq!(
            extractor.extract(&req).unwrap(),
            "2001:db8::1".parse::<IpAddr>().unwrap()
        );
    }
}
