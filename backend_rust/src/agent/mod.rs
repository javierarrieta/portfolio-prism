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
}

/// The TTL floor and ceiling, so a typo in a Secret cannot make approvals impossible or
/// permanent.
const TTL_RANGE: (i64, i64) = (1, 1440);

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            mcp_mount: true,
            batch_ttl_minutes: staging::DEFAULT_TTL_MINUTES,
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
        Self {
            mcp_mount,
            batch_ttl_minutes,
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
