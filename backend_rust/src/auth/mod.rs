//! Authentication configuration and mode selection.
//!
//! Two types describe the mode, because the mode is known at two different
//! times:
//!
//! * [`AuthRequest`] is the *parsed* `AUTH_MODE` value. It exists before the
//!   database does, so it cannot carry a user id.
//! * [`AuthMode`] is the *final* mode, after [`init_db`] has run. Its `Disabled`
//!   variant carries the id of the placeholder user, which only exists once the
//!   `users` table has been populated.
//!
//! Collapsing them would mean either reading the database before it is migrated,
//! or pretending a user id exists before it does.

pub mod fairing;
pub mod guard;
pub mod oidc;
pub mod routes;
pub mod session;

use sqlx::SqlitePool;

/// The parsed value of `AUTH_MODE`, known before a database exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthRequest {
    Oidc,
    Disabled,
}

/// OIDC client configuration, read from the environment.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    /// Derived from the origin of [`Self::redirect_uri`], not configured: the
    /// provider registers `https://<host>/` as a post-logout redirect, which is
    /// already implied by the callback URL. A sixth variable would be a second
    /// thing to get wrong.
    pub post_logout_redirect_uri: String,
    /// Authoritative copy of `BOOTSTRAP_OWNER_SUBJ`. [`StartupConfig`] keeps the
    /// same parsed value for `init_db`'s backfill; [`StartupConfig::from_env_with`]
    /// is the only place that fills both.
    pub bootstrap_subj: Option<String>,
}

/// Everything read from the environment before the database is touched.
#[derive(Debug, Clone)]
pub struct StartupConfig {
    pub requested: AuthRequest,
    pub oidc: Option<OidcConfig>,
    /// Copy of [`OidcConfig::bootstrap_subj`], kept here because `init_db` runs
    /// before an [`OidcConfig`] is handed to anything.
    pub bootstrap_subj: Option<String>,
}

/// The mode the running server enforces.
#[derive(Clone)]
pub enum AuthMode {
    Oidc(OidcConfig),
    /// Development escape hatch. Compiled out of a release binary, so a release
    /// build cannot run with authentication disabled even if the environment
    /// asks for it.
    #[cfg(debug_assertions)]
    Disabled {
        as_user: i32,
    },
}

impl StartupConfig {
    /// Read the configuration from the real environment.
    pub fn from_env() -> anyhow::Result<StartupConfig> {
        Self::from_env_with(&|key| std::env::var(key).ok())
    }

    /// The logic behind [`Self::from_env`], with the environment source injected.
    ///
    /// `std::env::set_var` is `unsafe` in edition 2024 and integration tests run
    /// multi-threaded, so the environment cannot be mutated to test refusals.
    /// Every rule lives here and takes the values as arguments instead.
    pub fn from_env_with(env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<StartupConfig> {
        let mode_raw = env("AUTH_MODE").map(|v| v.trim().to_string());
        let requested = match mode_raw.as_deref() {
            None | Some("") | Some("oidc") => AuthRequest::Oidc,
            Some("disabled") => AuthRequest::Disabled,
            Some(other) => anyhow::bail!(
                "AUTH_MODE={other:?} is not a valid value; expected `oidc` or `disabled`"
            ),
        };

        // `disabled` in a release build is a configuration error, not something
        // to silently ignore: the variant it needs is not compiled in, so the
        // operator would otherwise get a server that refuses to start for a
        // reason with no relation to what they set.
        if requested == AuthRequest::Disabled && !cfg!(debug_assertions) {
            anyhow::bail!(
                "AUTH_MODE=disabled is not available in a release build; \
                 the `disabled` mode is compiled out. Unset AUTH_MODE (it \
                 defaults to `oidc`) or run a debug build."
            );
        }

        // Trimmed, and empty-or-whitespace counts as unset. A set-but-empty value
        // would skip the OIDC refuse-to-start branch and then assign every orphan
        // to a placeholder no identity can ever claim — the misattribution the
        // bootstrap variable exists to prevent.
        let bootstrap_subj = env("BOOTSTRAP_OWNER_SUBJ")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        if requested != AuthRequest::Oidc {
            return Ok(StartupConfig {
                requested,
                oidc: None,
                bootstrap_subj: None,
            });
        }

        // Deny by default: there is no degraded anonymous mode, and naming every
        // missing variable at once beats four restarts.
        let mut missing: Vec<String> = Vec::new();
        let mut required = |key: &str| -> Option<String> {
            let value = env(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            if value.is_none() {
                missing.push(key.to_string());
            }
            value
        };
        let issuer = required("OIDC_ISSUER");
        let client_id = required("OIDC_CLIENT_ID");
        let client_secret = required("OIDC_CLIENT_SECRET");
        let redirect_uri = required("OIDC_REDIRECT_URI");
        if !missing.is_empty() {
            anyhow::bail!(
                "AUTH_MODE=oidc requires every of: {}; missing {missing:?}",
                [
                    "OIDC_ISSUER",
                    "OIDC_CLIENT_ID",
                    "OIDC_CLIENT_SECRET",
                    "OIDC_REDIRECT_URI"
                ]
                .join(", ")
            );
        }

        let redirect_uri = redirect_uri.unwrap();
        // `Secure` is set on the session cookie unconditionally (TLS terminates at
        // the ingress), so an http: redirect URI would both send the code in
        // cleartext and then loop with the cookie dropped by the browser.
        if !is_acceptable_redirect_uri(&redirect_uri) {
            anyhow::bail!(
                "OIDC_REDIRECT_URI={redirect_uri:?} must be https, or http to \
                 localhost/127.0.0.1 for local development"
            );
        }
        let issuer = issuer.unwrap();
        if !issuer.starts_with("https://") && !is_local_issuer(&issuer) {
            anyhow::bail!(
                "OIDC_ISSUER={issuer:?} must be https (or http to localhost for development)"
            );
        }

        let post_logout_redirect_uri = origin_of(&redirect_uri);

        Ok(StartupConfig {
            requested,
            oidc: Some(OidcConfig {
                issuer,
                client_id: client_id.unwrap(),
                client_secret: client_secret.unwrap(),
                redirect_uri,
                post_logout_redirect_uri,
                bootstrap_subj: bootstrap_subj.clone(),
            }),
            bootstrap_subj,
        })
    }
}

/// Turn the final [`AuthMode`] once the database is ready.
///
/// In `Disabled` mode the placeholder user must already exist, which is why
/// `init_db` creates it unconditionally. Reaching this error means the order of
/// operations in `main` is wrong, not that the operator misconfigured something.
pub async fn finalise_auth(pool: &SqlitePool, cfg: &StartupConfig) -> anyhow::Result<AuthMode> {
    // `pool` is read only by the `disabled` branch, which a release build compiles
    // out; without this the release build warns about an unused parameter.
    #[cfg(not(debug_assertions))]
    let _ = pool;
    match cfg.requested {
        AuthRequest::Oidc => match &cfg.oidc {
            Some(oidc) => Ok(AuthMode::Oidc(oidc.clone())),
            None => anyhow::bail!("AUTH_MODE=oidc but no OIDC configuration was parsed"),
        },
        #[cfg(debug_assertions)]
        AuthRequest::Disabled => {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT id FROM users WHERE issuer = 'local' AND subj = 'bootstrap'",
            )
            .fetch_optional(pool)
            .await?;
            match row {
                Some((id,)) => {
                    eprintln!(
                        "AUTH_MODE=disabled: every request runs as user id {id} \
                         (local/bootstrap). Do not use this configuration in production."
                    );
                    Ok(AuthMode::Disabled { as_user: id as i32 })
                }
                None => anyhow::bail!(
                    "AUTH_MODE=disabled needs the bootstrap placeholder user row, which \
                     init_db creates; this error means init_db did not run first"
                ),
            }
        }
        // Unreachable in a release build: `StartupConfig::from_env_with` refuses
        // `AUTH_MODE=disabled` before this is ever called, so no operator reaches
        // this arm by configuration. It exists because the *request* enum must keep
        // its `Disabled` variant in every profile — that is what lets the refusal
        // name the variable instead of failing to parse it.
        #[cfg(not(debug_assertions))]
        AuthRequest::Disabled => anyhow::bail!(
            "internal: `disabled` was requested in a release build, which \
             from_env_with is supposed to reject"
        ),
    }
}

/// `true` for `https://…`, and for `http://localhost[:port]` / `http://127.0.0.1[:port]`.
fn is_acceptable_redirect_uri(uri: &str) -> bool {
    uri.starts_with("https://") || is_local_issuer(uri)
}

fn is_local_issuer(uri: &str) -> bool {
    // Compare the *authority*, not a prefix: `starts_with("http://localhost")` is
    // satisfied by `http://localhost.attacker.example`, which would let a plain-http
    // issuer or redirect URI through the check that exists to keep http off the
    // network.
    let Some(rest) = uri.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if port.parse::<u16>().is_ok() => h,
        _ => authority,
    };
    host == "localhost" || host == "127.0.0.1"
}

/// The scheme + authority of `uri`, e.g. `https://app.example.com` from
/// `https://app.example.com/auth/callback`.
fn origin_of(uri: &str) -> String {
    match uri.find("://") {
        Some(idx) => {
            let rest = &uri[idx + 3..];
            let authority = rest.split('/').next().unwrap_or(rest);
            format!("{}://{}", &uri[..idx], authority)
        }
        None => uri.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an environment source from a fixed map.
    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        // Last write wins, so a test can override one entry of a base set.
        move |key: &str| {
            owned
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        }
    }

    fn full_oidc_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("OIDC_ISSUER", "https://auth.example.com/application/o/pp"),
            ("OIDC_CLIENT_ID", "portfolio-prism"),
            ("OIDC_CLIENT_SECRET", "s3cret"),
            ("OIDC_REDIRECT_URI", "https://pp.example.com/auth/callback"),
        ]
    }

    #[test]
    fn only_a_real_localhost_authority_passes_the_local_check() {
        // The check exists to let plain http off the loopback and nowhere else. A
        // prefix match satisfied it for any host *beginning* with `localhost`, which
        // is a hostname an attacker can register: `http://localhost.attacker.example`
        // is not local, and neither is anything else that merely starts with the word.
        for ok in [
            "http://localhost",
            "http://localhost:8080/.well-known/openid-configuration",
            "http://127.0.0.1:9000/iss",
        ] {
            assert!(is_local_issuer(ok), "{ok} should count as local");
        }
        for not_ok in [
            "http://localhost.attacker.example/",
            "http://localhostx:8080",
            "http://notlocalhost:8080",
            "http://127.0.0.1.evil.example",
            "http://localhost:notaport",
            "https://localhost",
        ] {
            assert!(!is_local_issuer(not_ok), "{not_ok} must not count as local");
        }
    }

    #[test]
    fn defaults_to_oidc_when_auth_mode_is_absent() {
        let cfg = StartupConfig::from_env_with(&env_from(&full_oidc_env())).unwrap();
        assert_eq!(cfg.requested, AuthRequest::Oidc);
        assert!(cfg.oidc.is_some());
    }

    #[test]
    fn post_logout_redirect_uri_is_the_origin_of_the_callback() {
        let cfg = StartupConfig::from_env_with(&env_from(&full_oidc_env())).unwrap();
        assert_eq!(
            cfg.oidc.unwrap().post_logout_redirect_uri,
            "https://pp.example.com"
        );
    }

    #[test]
    fn an_unknown_auth_mode_is_refused_naming_the_value() {
        let mut pairs = full_oidc_env();
        pairs.push(("AUTH_MODE", "magic"));
        let err = StartupConfig::from_env_with(&env_from(&pairs))
            .unwrap_err()
            .to_string();
        assert!(err.contains("AUTH_MODE") && err.contains("magic"), "{err}");
    }

    #[test]
    fn every_missing_oidc_variable_is_named_at_once() {
        let err = StartupConfig::from_env_with(&env_from(&[]))
            .unwrap_err()
            .to_string();
        for key in [
            "OIDC_ISSUER",
            "OIDC_CLIENT_ID",
            "OIDC_CLIENT_SECRET",
            "OIDC_REDIRECT_URI",
        ] {
            assert!(err.contains(key), "missing {key} from {err}");
        }
    }

    #[test]
    fn a_blank_oidc_variable_counts_as_missing() {
        let mut pairs = full_oidc_env();
        pairs.push(("OIDC_CLIENT_SECRET", "   "));
        let err = StartupConfig::from_env_with(&env_from(&pairs))
            .unwrap_err()
            .to_string();
        assert!(err.contains("OIDC_CLIENT_SECRET"), "{err}");
    }

    #[test]
    fn a_non_https_redirect_uri_is_refused_but_loopback_is_allowed() {
        let mut pairs = full_oidc_env();
        pairs.push(("OIDC_REDIRECT_URI", "http://pp.example.com/auth/callback"));
        assert!(StartupConfig::from_env_with(&env_from(&pairs)).is_err());

        pairs.push(("OIDC_REDIRECT_URI", "http://127.0.0.1:8000/auth/callback"));
        assert!(StartupConfig::from_env_with(&env_from(&pairs)).is_ok());
    }

    #[test]
    fn whitespace_bootstrap_subj_counts_as_unset() {
        let mut pairs = full_oidc_env();
        pairs.push(("BOOTSTRAP_OWNER_SUBJ", "  "));
        let cfg = StartupConfig::from_env_with(&env_from(&pairs)).unwrap();
        assert_eq!(cfg.bootstrap_subj, None);
        assert_eq!(cfg.oidc.unwrap().bootstrap_subj, None);
    }

    #[test]
    fn bootstrap_subj_is_the_same_value_on_both_configs() {
        let mut pairs = full_oidc_env();
        pairs.push(("BOOTSTRAP_OWNER_SUBJ", "authlib:42"));
        let cfg = StartupConfig::from_env_with(&env_from(&pairs)).unwrap();
        assert_eq!(cfg.bootstrap_subj.as_deref(), Some("authlib:42"));
        assert_eq!(
            cfg.oidc.unwrap().bootstrap_subj.as_deref(),
            Some("authlib:42")
        );
    }

    #[test]
    fn origin_of_handles_port_and_path() {
        assert_eq!(origin_of("https://a.b/x/y"), "https://a.b");
        assert_eq!(
            origin_of("http://localhost:8000/auth/callback"),
            "http://localhost:8000"
        );
    }
}
