#[derive(Debug, PartialEq)]
pub(super) enum HttpDenial {
    Host,
    Origin,
    Credential,
}

pub(super) fn authorize(
    port: u16,
    host: Option<&str>,
    origin: Option<&str>,
    authorization: Option<&str>,
    token: &str,
) -> Result<(), HttpDenial> {
    if !host.is_some_and(|host| {
        host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
    }) {
        return Err(HttpDenial::Host);
    }
    // Native MCP clients do not need a browser origin or a CORS exception.
    if origin.is_some() {
        return Err(HttpDenial::Origin);
    }
    use subtle::ConstantTimeEq;
    let supplied = authorization
        .and_then(|header| header.strip_prefix("Bearer "))
        .unwrap_or_default();
    if token.len() != 64 || !bool::from(supplied.as_bytes().ct_eq(token.as_bytes())) {
        return Err(HttpDenial::Credential);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    #[test]
    fn stored_connection_is_one_key_without_clients_or_scopes() {
        let policy: AccessPolicy = serde_json::from_value(serde_json::json!({
            "schema":2,"storage_profile_id":"profile-a","enabled":false,"port":38192,"token":TOKEN,
        }))
        .expect("Single-key MCP settings must decode");
        policy.validate_for_profile("profile-a").unwrap();
        let encoded = serde_json::to_value(policy).unwrap();
        assert_eq!(encoded["token"], TOKEN);
        assert!(encoded.get("grants").is_none());
        assert!(encoded.get("permissions").is_none());
    }
    #[test]
    fn local_http_rejects_missing_or_wrong_credentials() {
        assert_eq!(
            authorize(38192, Some("127.0.0.1:38192"), None, None, TOKEN),
            Err(HttpDenial::Credential)
        );
        assert_eq!(
            authorize(
                38192,
                Some("127.0.0.1:38192"),
                None,
                Some("Bearer other"),
                TOKEN
            ),
            Err(HttpDenial::Credential)
        );
    }
    #[test]
    fn local_http_rejects_rebinding_and_browser_origins() {
        let auth = format!("Bearer {TOKEN}");
        for host in [None, Some("example.org:38192"), Some("127.0.0.1:7")] {
            assert_eq!(
                authorize(38192, host, None, Some(&auth), TOKEN),
                Err(HttpDenial::Host)
            );
        }
        for origin in ["https://example.org", "null", "http://127.0.0.1:38192"] {
            assert_eq!(
                authorize(
                    38192,
                    Some("127.0.0.1:38192"),
                    Some(origin),
                    Some(&auth),
                    TOKEN
                ),
                Err(HttpDenial::Origin)
            );
        }
    }
    #[test]
    fn authorized_native_client_can_connect_with_exact_local_authority() {
        let auth = format!("Bearer {TOKEN}");
        for host in ["127.0.0.1:38192", "localhost:38192"] {
            assert_eq!(
                authorize(38192, Some(host), None, Some(&auth), TOKEN),
                Ok(())
            );
        }
    }
    #[test]
    fn copied_authority_cannot_enable_a_different_storage_profile() {
        let policy = AccessPolicy {
            storage_profile_id: "profile-a".into(),
            token: TOKEN.into(),
            ..Default::default()
        };
        assert!(policy.validate_for_profile("profile-a").is_ok());
        assert!(policy.validate_for_profile("profile-b").is_err());
    }

    #[test]
    fn one_key_survives_off_on_port_changes_and_restarts() {
        let mut generated = 0;
        let (mut policy, write) = AccessPolicy::restore(None, "profile-a", || {
            generated += 1;
            Ok(TOKEN.into())
        })
        .unwrap();
        assert!(write && !policy.enabled);
        for (enabled, port) in [(true, 38192), (false, 41000), (true, 41000)] {
            policy.enabled = enabled;
            policy.port = port;
            let bytes = serde_json::to_vec(&policy).unwrap();
            let (restored, write) = AccessPolicy::restore(Some(&bytes), "profile-a", || {
                panic!("A persisted key must not be regenerated")
            })
            .unwrap();
            assert!(!write);
            assert_eq!(restored.token, TOKEN);
            assert_eq!(restored.enabled, enabled);
            assert_eq!(restored.port, port);
            policy = restored;
        }
        assert_eq!(generated, 1);
    }

    #[test]
    fn legacy_clients_are_replaced_by_a_fresh_disabled_connection() {
        let bytes=serde_json::to_vec(&serde_json::json!({"schema":1,"storage_profile_id":"profile-a","enabled":true,"port":40001,"grants":[{"token":TOKEN,"permissions":{"manage":false}}]})).unwrap();
        let new_token = "b".repeat(64);
        let (policy, write) =
            AccessPolicy::restore(Some(&bytes), "profile-a", || Ok(new_token.clone())).unwrap();
        assert!(write && !policy.enabled);
        assert_eq!(policy.port, 40001);
        assert_eq!(policy.token, new_token);
        assert_eq!(
            authorize(
                40001,
                Some("127.0.0.1:40001"),
                None,
                Some(&format!("Bearer {TOKEN}")),
                &policy.token
            ),
            Err(HttpDenial::Credential)
        );
    }

    #[test]
    fn invalid_foreign_and_future_settings_do_not_generate_or_overwrite_a_key() {
        for value in [
            serde_json::json!({"schema":2,"storage_profile_id":"profile-a","enabled":true,"port":38192,"token":"broken"}),
            serde_json::json!({"schema":2,"storage_profile_id":"other","enabled":true,"port":38192,"token":TOKEN}),
            serde_json::json!({"schema":3,"storage_profile_id":"profile-a","enabled":true,"port":38192,"token":TOKEN}),
            serde_json::json!({"schema":1,"storage_profile_id":"other","enabled":true,"port":38192,"grants":[]}),
        ] {
            let bytes = serde_json::to_vec(&value).unwrap();
            assert!(AccessPolicy::restore(Some(&bytes), "profile-a", || panic!(
                "Invalid data must not be replaced with a new key"
            ))
            .is_err());
        }
    }
}
use serde::{Deserialize, Serialize};

pub(super) const DEFAULT_PORT: u16 = 38192;
pub(super) const LOCAL_AGENT_ID: &str = "local-agent";

#[derive(Clone)]
pub(super) struct AgentAccess {
    pub id: String,
    pub token: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccessPolicy {
    pub schema: u32,
    pub storage_profile_id: String,
    pub enabled: bool,
    pub port: u16,
    pub token: String,
}
impl Default for AccessPolicy {
    fn default() -> Self {
        Self {
            schema: 2,
            storage_profile_id: String::new(),
            enabled: false,
            port: DEFAULT_PORT,
            token: String::new(),
        }
    }
}
impl AccessPolicy {
    pub fn agent(&self) -> AgentAccess {
        AgentAccess {
            id: LOCAL_AGENT_ID.into(),
            token: self.token.clone(),
        }
    }

    /// Return whether the loaded/new connection must be persisted before publication.
    pub fn restore(
        bytes: Option<&[u8]>,
        profile: &str,
        mut generate: impl FnMut() -> Result<String, String>,
    ) -> Result<(Self, bool), String> {
        let port = if let Some(bytes) = bytes {
            #[derive(Deserialize)]
            struct Version {
                schema: u32,
            }
            let version: Version =
                serde_json::from_slice(bytes).map_err(|_| "Invalid protected MCP settings")?;
            if version.schema == 2 {
                let policy: Self =
                    serde_json::from_slice(bytes).map_err(|_| "Invalid protected MCP settings")?;
                policy.validate_for_profile(profile)?;
                return Ok((policy, false));
            }
            if version.schema != 1 {
                return Err("Unsupported MCP settings version".into());
            }
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Legacy {
                #[serde(rename = "schema")]
                _schema: u32,
                storage_profile_id: String,
                #[serde(rename = "enabled")]
                _enabled: bool,
                port: u16,
                #[serde(rename = "grants")]
                _grants: serde::de::IgnoredAny,
            }
            let legacy: Legacy =
                serde_json::from_slice(bytes).map_err(|_| "Invalid protected MCP settings")?;
            if legacy.storage_profile_id != profile {
                return Err("MCP settings belong to a different storage profile".into());
            }
            // Restricted legacy tokens must never silently acquire full authority.
            legacy.port
        } else {
            DEFAULT_PORT
        };
        if port < 1024 {
            return Err("Invalid MCP port".into());
        }
        let policy = Self {
            storage_profile_id: profile.into(),
            port,
            token: generate()?,
            ..Default::default()
        };
        policy.validate_for_profile(profile)?;
        Ok((policy, true))
    }

    pub fn validate_for_profile(&self, expected: &str) -> Result<(), String> {
        self.validate()?;
        if self.storage_profile_id != expected {
            return Err("MCP settings belong to a different storage profile".into());
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 2
            || self.port < 1024
            || self.token.len() != 64
            || !self.token.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Invalid MCP settings version, port or connection key".into());
        }
        Ok(())
    }
}
