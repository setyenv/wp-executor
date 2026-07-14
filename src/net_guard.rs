//! Outbound egress guard (SSRF protection) for capabilities that make network
//! requests (currently `http.request`).
//!
//! The guard is **on by default**: a destination is rejected when it resolves
//! to a non-global address — RFC-1918 private ranges, loopback, link-local
//! (including the cloud metadata endpoint `169.254.169.254`), IPv6 unique-local
//! and link-local, IPv4-mapped forms of any of those, and a few other
//! special-use ranges. The host is resolved and every resulting IP is checked
//! (not just the URL string), and the validated address is pinned for the
//! actual connection, so a name that resolves public-then-private (DNS
//! rebinding) cannot slip through.
//!
//! It is **relaxable** via an operator-defined allowlist of hosts/patterns
//! (`allowed_egress_hosts`): a matching host is exempt from the block, so
//! specific internal destinations the operator trusts are reachable. A single
//! entry `"*"` disables the guard entirely.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// True when `ip` must NOT be reached from a guarded egress context — i.e. it
/// is private/internal/special-use rather than a globally-routable address.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            // ::ffff:a.b.c.d — classify by the embedded IPv4 address.
            Some(mapped) => is_blocked_v4(mapped),
            None => is_blocked_v6(v6),
        },
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    if ip.is_private()          // 10/8, 172.16/12, 192.168/16
        || ip.is_loopback()     // 127/8
        || ip.is_link_local()   // 169.254/16 (incl. 169.254.169.254 metadata)
        || ip.is_broadcast()    // 255.255.255.255
        || ip.is_documentation()// 192.0.2/24, 198.51.100/24, 203.0.113/24
        || ip.is_unspecified()  // 0.0.0.0
        || ip.is_multicast()
    // 224/4
    {
        return true;
    }
    let o = ip.octets();
    o[0] == 0                                  // 0.0.0.0/8 "this network"
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 CGNAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24 IETF protocol assignments
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15 benchmarking
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback()         // ::1
        || ip.is_unspecified()  // ::
        || ip.is_multicast()
    // ff00::/8
    {
        return true;
    }
    let seg0 = ip.segments()[0];
    (seg0 & 0xfe00) == 0xfc00    // fc00::/7 unique local
        || (seg0 & 0xffc0) == 0xfe80 // fe80::/10 link-local
}

/// Does `host` match any pattern in `allowlist`?
///
/// - `"*"` matches everything (guard disabled).
/// - `"*.example.com"` or `".example.com"` matches `example.com` and any
///   subdomain of it.
/// - anything else is an exact, case-insensitive host match.
pub fn host_in_allowlist(host: &str, allowlist: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allowlist.iter().any(|pat| {
        let p = pat.trim().to_ascii_lowercase();
        if p == "*" {
            return true;
        }
        let suffix = p.strip_prefix("*.").or_else(|| p.strip_prefix('.'));
        match suffix {
            Some(s) => host == s || host.ends_with(&format!(".{}", s)),
            None => host == p,
        }
    })
}

/// The egress guard, parameterised by the operator allowlist.
pub struct EgressGuard<'a> {
    allowlist: &'a [String],
}

impl<'a> EgressGuard<'a> {
    pub fn new(allowlist: &'a [String]) -> Self {
        Self { allowlist }
    }

    /// The guard enforces unless the allowlist opts out entirely with `"*"`.
    pub fn enforcing(&self) -> bool {
        !self.allowlist.iter().any(|p| p.trim() == "*")
    }

    fn host_allowed(&self, host: &str) -> bool {
        host_in_allowlist(host, self.allowlist)
    }

    /// Validate `host:port` for egress.
    ///
    /// - `Ok(None)`        — host is allowlisted: connect normally, no pinning.
    /// - `Ok(Some(addrs))` — validated global address(es) to pin the connection to.
    /// - `Err(msg)`        — blocked by the SSRF guard.
    pub async fn resolve_checked(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Option<Vec<SocketAddr>>, String> {
        if self.host_allowed(host) {
            return Ok(None);
        }

        // IP literal: validate directly, no DNS.
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked_ip(ip) {
                return Err(blocked_msg(host, ip));
            }
            return Ok(Some(vec![SocketAddr::new(ip, port)]));
        }

        // Hostname: resolve and validate EVERY address it maps to.
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| format!("egress: cannot resolve host '{}': {}", host, e))?
            .collect();
        if addrs.is_empty() {
            return Err(format!("egress: host '{}' did not resolve", host));
        }
        for a in &addrs {
            if is_blocked_ip(a.ip()) {
                return Err(blocked_msg(host, a.ip()));
            }
        }
        Ok(Some(addrs))
    }
}

fn blocked_msg(host: &str, ip: IpAddr) -> String {
    format!(
        "egress blocked: '{}' resolves to non-global address {} (SSRF guard). \
         Add the host to allowed_egress_hosts to permit it.",
        host, ip
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blocks_private_and_special_v4() {
        for s in [
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "127.0.0.1",
            "169.254.0.1",
            "169.254.169.254", // cloud metadata
            "0.0.0.0",
            "100.64.0.1", // CGNAT
            "198.18.0.1", // benchmarking
            "255.255.255.255",
            "224.0.0.1", // multicast
        ] {
            assert!(is_blocked_ip(ip(s)), "expected {} to be blocked", s);
        }
    }

    #[test]
    fn allows_public_v4() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "172.15.0.1",
            "172.32.0.1",
        ] {
            assert!(!is_blocked_ip(ip(s)), "expected {} to be allowed", s);
        }
    }

    #[test]
    fn blocks_private_and_special_v6() {
        for s in [
            "::1",             // loopback
            "::",              // unspecified
            "fc00::1",         // unique local
            "fd00::1",         // unique local
            "fe80::1",         // link local
            "ff02::1",         // multicast
            "::ffff:10.0.0.1", // IPv4-mapped private
            "::ffff:169.254.169.254",
        ] {
            assert!(is_blocked_ip(ip(s)), "expected {} to be blocked", s);
        }
    }

    #[test]
    fn allows_public_v6() {
        for s in ["2606:4700:4700::1111", "2001:4860:4860::8888"] {
            assert!(!is_blocked_ip(ip(s)), "expected {} to be allowed", s);
        }
    }

    #[test]
    fn allowlist_matching() {
        let list = vec![
            "api.example.com".to_string(),
            "*.internal.test".to_string(),
            ".corp.example".to_string(),
        ];
        assert!(host_in_allowlist("api.example.com", &list));
        assert!(host_in_allowlist("API.EXAMPLE.COM", &list)); // case-insensitive
        assert!(host_in_allowlist("db.internal.test", &list)); // wildcard subdomain
        assert!(host_in_allowlist("internal.test", &list)); // wildcard apex
        assert!(host_in_allowlist("a.b.corp.example", &list)); // leading-dot suffix
        assert!(!host_in_allowlist("example.com", &list));
        assert!(!host_in_allowlist("evil.com", &list));
    }

    #[test]
    fn wildcard_disables_enforcement() {
        let star = vec!["*".to_string()];
        assert!(!EgressGuard::new(&star).enforcing());
        let empty: Vec<String> = vec![];
        assert!(EgressGuard::new(&empty).enforcing());
    }

    #[tokio::test]
    async fn resolve_checked_blocks_ip_literal() {
        let empty: Vec<String> = vec![];
        let g = EgressGuard::new(&empty);
        assert!(g.resolve_checked("169.254.169.254", 80).await.is_err());
        assert!(g.resolve_checked("10.0.0.1", 80).await.is_err());
    }

    #[tokio::test]
    async fn resolve_checked_allows_public_ip_literal() {
        let empty: Vec<String> = vec![];
        let g = EgressGuard::new(&empty);
        let addrs = g.resolve_checked("8.8.8.8", 443).await.unwrap().unwrap();
        assert_eq!(addrs, vec!["8.8.8.8:443".parse::<SocketAddr>().unwrap()]);
    }

    #[tokio::test]
    async fn resolve_checked_exempts_allowlisted_host() {
        let list = vec!["127.0.0.1".to_string()];
        let g = EgressGuard::new(&list);
        // Allowlisted -> Ok(None) even though the address is loopback.
        assert!(g.resolve_checked("127.0.0.1", 80).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_checked_blocks_localhost_name() {
        // Exercises the resolve-then-validate path with a name that maps to a
        // loopback address (hermetic: localhost resolves locally).
        let empty: Vec<String> = vec![];
        let g = EgressGuard::new(&empty);
        assert!(g.resolve_checked("localhost", 80).await.is_err());
    }
}
