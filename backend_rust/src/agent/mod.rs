//! The agent module: everything that exists because a model, not a person, is on the other
//! end of the request.
//!
//! `protocol` is what an agent may send, `staging` is the lifecycle that turns a proposal
//! into rows only after a human has said so, `tools` is what an agent may call, and `mount`
//! is the door they all arrive through.
//!
//! Reading order that matches the trust model: `staging` holds the state, `tools` is thin
//! over it, `mount` decides who gets to reach `tools` at all.
//!
//! The invariant the rest of the design leans on: **the write and the decision to write are
//! different requests with different credentials.** An agent holds a token and can stage;
//! only a session cookie can approve. That is why the state lives in `agent_staged_batch`
//! and not in the conversation — a status the model can describe is a status the model can
//! invent.

pub mod mount;
pub mod protocol;
pub mod staging;
pub mod tools;

/// What the operator decides about the agent surface, read once at startup.
///
/// This exists because the spec documented two levers and the code had neither: the route
/// was mounted unconditionally, so there was no way to turn the agent surface off without
/// rolling a different image, and the batch TTL was a `const`. A documented lever that is
/// not wired is worse than no lever, because an operator who sets it gets silence.
///
/// Threaded through `build_rocket` rather than read inside a handler, for the same reason
/// `BackupConfig` is (`tests/common/mod.rs` spells it out): `std::env::set_var` is `unsafe`
/// in edition 2024 and racy across parallel tests, so the only testable way in is a
/// parameter.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Whether `/mcp` is mounted at all. **Default: mounted.**
    ///
    /// Default-on rather than default-off, because the deployed topology reaches the agent
    /// surface through nginx (`frontend/nginx.conf`, `location /mcp`) and a default-off
    /// build would make the feature silently absent for anyone who did not read this file.
    /// The lever is a kill switch, not an opt-in: set `AGENT_MCP_MOUNT=0` to take the
    /// surface down without a rebuild.
    pub mcp_mount: bool,
    /// Minutes a staged batch stays approvable before it expires. Default 60, clamped to
    /// 1..=1440 — a batch that expires in zero minutes can never be approved by a human
    /// who is away from the keyboard, and one that lasts longer than a day is not a
    /// pending approval, it is a standing write permission.
    pub batch_ttl_minutes: i64,
    /// The `Host` authorities `/mcp` will answer, as that header arrives **at this
    /// process**. Behind the shipped nginx that is the public hostname, not loopback:
    /// `frontend/nginx.conf` proxies `/mcp` with `proxy_set_header Host $host`, so the
    /// value to put here is the name the browser types, without a scheme.
    ///
    /// **Default: loopback only** — `localhost`, `127.0.0.1`, `::1`. That is rmcp's own
    /// default and it is a local-development default, and leaving it in place on a real
    /// deployment is not a silent failure but a misleading one: the guard runs *after* the
    /// bearer check, so every call answers `403 Forbidden: Host header is not allowed`
    /// against a token that is perfectly valid. It reads as a broken token, and nothing in
    /// the response says otherwise. Any deployment reached by a name other than the one
    /// making the request has to name itself here.
    ///
    /// An entry with no port matches any port, so the bare hostname is the right shape for
    /// a proxy-fronted deployment; an entry with a port matches that port and nothing else.
    /// Matching is case-insensitive and IPv6 brackets are ignored, both done by rmcp.
    pub mcp_allowed_hosts: Vec<String>,
}

/// The TTL floor and ceiling, so a typo in a Secret cannot make approvals impossible or
/// permanent.
const TTL_RANGE: (i64, i64) = (1, 1440);

/// rmcp's loopback-only default, read from the library rather than restated here, so the
/// fallback cannot drift from the guard it is falling back to.
fn default_mcp_allowed_hosts() -> Vec<String> {
    rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default().allowed_hosts
}

/// Reduce one `MCP_ALLOWED_HOSTS` entry to a bare authority.
///
/// The shape this exists for is the mistake an operator actually makes: `ALLOWED_ORIGINS`
/// lives beside this variable in the same ConfigMap and is a **URL**
/// (`https://portfolio.example.com`), while this one wants an **authority**
/// (`portfolio.example.com`). Copying the format across is a 403 on every MCP call with no
/// hint that a configuration value is at fault, so the scheme and any path are taken off
/// here and the correction is printed rather than swallowed.
fn normalize_host_entry(raw: &str) -> Option<String> {
    let mut entry = raw.trim();
    if let Some((_, rest)) = entry.split_once("://") {
        entry = rest;
    }
    // Split on `/` only. Splitting on `:` would take the tail off an IPv6 literal.
    entry = entry.split('/').next().unwrap_or(entry).trim();
    if entry.is_empty() {
        None
    } else {
        Some(entry.to_string())
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            mcp_mount: true,
            batch_ttl_minutes: staging::DEFAULT_TTL_MINUTES,
            mcp_allowed_hosts: default_mcp_allowed_hosts(),
        }
    }
}

impl AgentConfig {
    pub fn from_env() -> Self {
        Self::from_env_with(&|key| std::env::var(key).ok())
    }

    pub fn from_env_with(env: &dyn Fn(&str) -> Option<String>) -> Self {
        let defaults = Self::default();
        let mcp_mount = match env("AGENT_MCP_MOUNT") {
            // Absent means default. Empty means absent-in-a-Secret, which is how a
            // half-filled manifest actually looks.
            None => defaults.mcp_mount,
            Some(v) => match v.trim().to_ascii_lowercase().as_str() {
                "" | "1" | "true" | "yes" | "on" => true,
                "0" | "false" | "no" | "off" => false,
                // A value that is neither is a typo, and guessing which way a typo points
                // is how a kill switch stops being one. Default, and say so.
                other => {
                    eprintln!("WARN: AGENT_MCP_MOUNT={other:?} is not recognised; using the default (mounted)");
                    defaults.mcp_mount
                }
            },
        };
        let batch_ttl_minutes = match env("AGENT_BATCH_TTL_MINUTES") {
            None => defaults.batch_ttl_minutes,
            Some(v) => match v.trim().parse::<i64>() {
                Ok(n) if (TTL_RANGE.0..=TTL_RANGE.1).contains(&n) => n,
                Ok(n) => {
                    eprintln!(
                        "WARN: AGENT_BATCH_TTL_MINUTES={n} is outside {}..={}; using {}",
                        TTL_RANGE.0, TTL_RANGE.1, defaults.batch_ttl_minutes
                    );
                    defaults.batch_ttl_minutes
                }
                Err(_) => {
                    eprintln!(
                        "WARN: AGENT_BATCH_TTL_MINUTES={v:?} is not an integer; using {}",
                        defaults.batch_ttl_minutes
                    );
                    defaults.batch_ttl_minutes
                }
            },
        };
        let mcp_allowed_hosts = match env("MCP_ALLOWED_HOSTS") {
            None => defaults.mcp_allowed_hosts.clone(),
            Some(v) if v.trim() == "*" => {
                // The only way to say "any Host" is rmcp's empty list, and an empty list
                // means exactly that: the DNS-rebinding guard is off. Loud, because it is
                // the one value here that removes a protection rather than configuring it.
                eprintln!(
                    "WARN: MCP_ALLOWED_HOSTS=* accepts any Host header; the DNS-rebinding guard on /mcp is disabled"
                );
                Vec::new()
            }
            Some(v) => {
                let mut hosts: Vec<String> = Vec::new();
                for raw in v.split(',') {
                    let raw = raw.trim();
                    let Some(host) = normalize_host_entry(raw) else {
                        continue;
                    };
                    if host != raw {
                        eprintln!(
                            "WARN: MCP_ALLOWED_HOSTS entry {raw:?} is not a bare authority; matching on {host:?}"
                        );
                    }
                    hosts.push(host);
                }
                if hosts.is_empty() {
                    // Not the same thing as `*`. An empty list in rmcp means *allow every
                    // host*, so a value that parses to nothing — `""`, `" , , "` — must
                    // never reach it. That is how a half-filled Secret would open the guard
                    // instead of leaving it alone.
                    eprintln!(
                        "WARN: MCP_ALLOWED_HOSTS={v:?} names no host; using the default (loopback only)"
                    );
                    defaults.mcp_allowed_hosts.clone()
                } else {
                    hosts
                }
            }
        };
        Self {
            mcp_mount,
            batch_ttl_minutes,
            mcp_allowed_hosts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_with(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn defaults_mount_the_route_and_use_the_documented_ttl() {
        let cfg = AgentConfig::from_env_with(&env_with(&[]));
        assert!(cfg.mcp_mount, "unconfigured must mount, or the surface is dark by default");
        assert_eq!(cfg.batch_ttl_minutes, staging::DEFAULT_TTL_MINUTES);
    }

    #[test]
    fn an_unrecognised_mount_value_does_not_silently_disable_the_surface() {
        // The point of this case: `ture` must not read as off. A kill switch that guesses
        // is a kill switch that fires on a typo.
        for v in ["ture", "yes-pls", "maybe"] {
            let cfg = AgentConfig::from_env_with(&env_with(&[("AGENT_MCP_MOUNT", v)]));
            assert!(cfg.mcp_mount, "{v:?} must fall back to the default (mounted)");
        }
    }

    #[test]
    fn the_documented_falsy_values_take_the_surface_down() {
        for v in ["0", "false", "no", "off", " OFF "] {
            let cfg = AgentConfig::from_env_with(&env_with(&[("AGENT_MCP_MOUNT", v)]));
            assert!(!cfg.mcp_mount, "{v:?} must unmount");
        }
    }

    #[test]
    fn an_unset_host_allowlist_is_the_library_loopback_default_not_every_host() {
        // The defect this variable exists for: the default is right for a laptop and wrong
        // for a deployment, and the failure it produces is a 403 that names the token's
        // host rather than the missing configuration. So the default is pinned, not left
        // to whatever the dependency happens to ship.
        let cfg = AgentConfig::from_env_with(&env_with(&[]));
        assert_eq!(
            cfg.mcp_allowed_hosts,
            default_mcp_allowed_hosts(),
            "unset must fall back to rmcp's own loopback list"
        );
        assert!(
            !cfg.mcp_allowed_hosts.is_empty(),
            "an empty allowlist means allow-any-host, and unset must never mean that"
        );
    }

    #[test]
    fn the_host_allowlist_is_a_list_and_empty_entries_are_not_a_wildcard() {
        let cfg = AgentConfig::from_env_with(&env_with(&[(
            "MCP_ALLOWED_HOSTS",
            " portfolio.example.com , , 10.0.0.5:8000 ,",
        )]));
        assert_eq!(
            cfg.mcp_allowed_hosts,
            vec!["portfolio.example.com".to_string(), "10.0.0.5:8000".to_string()]
        );
        // Empty and comma-only are the half-filled-Secret shape. Both must land on the
        // default, because in rmcp an empty list is the opposite of a no-op.
        for blank in ["", "   ", ",", " , , "] {
            let cfg = AgentConfig::from_env_with(&env_with(&[("MCP_ALLOWED_HOSTS", blank)]));
            assert_eq!(
                cfg.mcp_allowed_hosts,
                default_mcp_allowed_hosts(),
                "{blank:?} must fall back rather than allow every host"
            );
        }
    }

    #[test]
    fn a_url_shaped_host_entry_is_reduced_to_an_authority() {
        // `ALLOWED_ORIGINS` is a URL and sits next to this in the ConfigMap; copying its
        // format across is the likeliest misconfiguration and the one that fails silently.
        for (raw, want) in [
            ("https://portfolio.example.com", "portfolio.example.com"),
            ("https://portfolio.example.com/", "portfolio.example.com"),
            ("http://portfolio.example.com:8443/x", "portfolio.example.com:8443"),
            ("portfolio.example.com", "portfolio.example.com"),
        ] {
            let cfg = AgentConfig::from_env_with(&env_with(&[("MCP_ALLOWED_HOSTS", raw)]));
            assert_eq!(cfg.mcp_allowed_hosts, vec![want.to_string()], "{raw:?}");
        }
    }

    #[test]
    fn only_a_lone_star_disables_the_guard() {
        let cfg = AgentConfig::from_env_with(&env_with(&[("MCP_ALLOWED_HOSTS", " * ")]));
        assert!(
            cfg.mcp_allowed_hosts.is_empty(),
            "`*` is the documented off switch and an empty list is how rmcp spells it"
        );
        // Anything else keeps the guard. A star inside a list is a typo for a hostname, not
        // a request to open the endpoint, and must not become the empty list.
        let cfg = AgentConfig::from_env_with(&env_with(&[("MCP_ALLOWED_HOSTS", "*,other")]))
            ;
        assert_eq!(cfg.mcp_allowed_hosts, vec!["*".to_string(), "other".to_string()]);
    }

    #[test]
    fn ttl_is_parsed_and_clamped_to_a_range_a_human_can_use() {
        let cfg = AgentConfig::from_env_with(&env_with(&[("AGENT_BATCH_TTL_MINUTES", "5")]))
            ;
        assert_eq!(cfg.batch_ttl_minutes, 5);
        for bad in ["0", "-1", "1441", "99999", "60min", ""] {
            let cfg = AgentConfig::from_env_with(&env_with(&[("AGENT_BATCH_TTL_MINUTES", bad)]))
                ;
            assert_eq!(
                cfg.batch_ttl_minutes,
                staging::DEFAULT_TTL_MINUTES,
                "{bad:?} must fall back rather than make approvals impossible or permanent"
            );
        }
    }
}
