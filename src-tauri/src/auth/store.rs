//! The on-disk Authentik session.
//!
//! Extracted with `openid profile email offline_access`, so the access token
//! lasts an hour and the refresh token 30 days (rotating on every use). The
//! file is `~/.config/clipforge/auth.json`, created 0600 through a temp file and
//! an atomic rename. A missing or corrupt file simply means "signed out".

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::model::AuthStatus;
use crate::settings::config_dir;

/// Refresh this long before expiry instead of waiting for a 401 mid-upload.
pub const REFRESH_MARGIN_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    pub sub: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
}

pub struct AuthStore {
    path: PathBuf,
    inner: Mutex<Option<StoredTokens>>,
    /// Serialises refreshes: rotating refresh tokens make concurrent refreshes
    /// actively harmful (the second one invalidates the first).
    pub(crate) refresh_lock: tokio::sync::Mutex<()>,
}

impl AuthStore {
    /// The live store, backed by `~/.config/clipforge/auth.json`.
    pub fn load() -> Arc<Self> {
        Arc::new(Self::from_path(default_path()))
    }

    fn from_path(path: PathBuf) -> Self {
        let tokens = read(&path);
        Self {
            path,
            inner: Mutex::new(tokens),
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn get(&self) -> Option<StoredTokens> {
        self.inner.lock().clone()
    }

    pub fn is_signed_in(&self) -> bool {
        self.get().is_some()
    }

    /// What the frontend is allowed to see. Tokens are deliberately absent.
    pub fn status(&self) -> AuthStatus {
        match self.get() {
            Some(tokens) => AuthStatus {
                signed_in: true,
                name: tokens.name,
                email: tokens.email,
                sub: tokens.sub,
                expires_at: rfc3339(tokens.expires_at),
            },
            None => AuthStatus {
                signed_in: false,
                name: None,
                email: None,
                sub: None,
                expires_at: None,
            },
        }
    }

    /// Persist `tokens` first, then make them live. A failed write leaves the
    /// previous session untouched.
    pub fn put(&self, tokens: StoredTokens) -> Result<()> {
        let mut inner = self.inner.lock();
        self.persist(Some(&tokens))?;
        *inner = Some(tokens);
        Ok(())
    }

    /// Remove the session from memory and disk.
    pub fn clear(&self) -> Result<()> {
        let mut inner = self.inner.lock();
        self.persist(None)?;
        *inner = None;
        Ok(())
    }

    fn persist(&self, tokens: Option<&StoredTokens>) -> Result<()> {
        let Some(tokens) = tokens else {
            return match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(AppError::msg(format!(
                    "Could not remove {}: {err}",
                    self.path.display()
                ))),
            };
        };

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_vec_pretty(tokens)?;
        let temp = self.path.with_extension("json.tmp");

        let written = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temp)?;
            // `mode` only applies when the file is created; enforce it in case
            // an earlier attempt left the temp file with looser permissions.
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            file.write_all(&json)?;
            file.sync_all()?;
            Ok(())
        })();

        if let Err(err) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(AppError::msg(format!(
                "Could not write the Authentik sign-in to {}: {err}",
                self.path.display()
            )));
        }

        if let Err(err) = std::fs::rename(&temp, &self.path) {
            let _ = std::fs::remove_file(&temp);
            return Err(AppError::msg(format!(
                "Could not store the Authentik sign-in at {}: {err}",
                self.path.display()
            )));
        }

        // A pre-existing auth.json may have been created with looser bits.
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}

fn read(path: &Path) -> Option<StoredTokens> {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<StoredTokens>(&raw) {
            Ok(tokens) => Some(tokens),
            Err(err) => {
                // Never log the file contents: they are tokens.
                log::warn!(
                    "{} is not a valid Authentik sign-in; treating it as signed out: {err}",
                    path.display()
                );
                None
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            log::warn!("could not read {}: {err}", path.display());
            None
        }
    }
}

/// The access token is close enough to expiry that it should be refreshed
/// before use.
pub fn needs_refresh(now_secs: i64, expires_at: i64) -> bool {
    expires_at.saturating_sub(now_secs) <= REFRESH_MARGIN_SECS
}

fn rfc3339(secs: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn default_path() -> PathBuf {
    config_dir().join("auth.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "clipforge-auth-{}-{label}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    fn tokens(now: i64) -> StoredTokens {
        StoredTokens {
            access_token: "ACCESS-SECRET".into(),
            refresh_token: Some("REFRESH-SECRET".into()),
            expires_at: now + 3600,
            sub: Some("mafio".into()),
            name: Some("Maf".into()),
            email: Some("maf@example.com".into()),
        }
    }

    #[test]
    fn tokens_round_trip_and_the_file_is_private() {
        let path = temp_path("round-trip");
        let store = AuthStore::from_path(path.clone());
        assert!(!store.is_signed_in());

        store.put(tokens(1_000_000)).expect("put");
        assert!(store.is_signed_in());

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "auth.json must be readable only by the user");

        let reopened = AuthStore::from_path(path.clone());
        assert_eq!(reopened.get(), Some(tokens(1_000_000)));

        store.clear().expect("clear");
        assert!(!store.is_signed_in());
        assert!(!path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_means_signed_out() {
        let path = temp_path("missing");
        let store = AuthStore::from_path(path.clone());
        assert!(!store.is_signed_in());
        assert!(!path.exists());
        let status = store.status();
        assert!(!status.signed_in);
        assert!(status.name.is_none());
    }

    #[test]
    fn a_corrupt_file_means_signed_out_and_is_repaired_on_the_next_sign_in() {
        let path = temp_path("corrupt");
        std::fs::write(&path, b"not json and never shall be").expect("write");

        let store = AuthStore::from_path(path.clone());
        assert!(!store.is_signed_in());

        store.put(tokens(2_000_000)).expect("put");
        let reopened = AuthStore::from_path(path.clone());
        assert!(reopened.is_signed_in());
        assert_eq!(
            reopened.get().expect("tokens").sub.as_deref(),
            Some("mafio")
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_status_sent_to_the_frontend_never_contains_tokens() {
        let path = temp_path("status");
        let store = AuthStore::from_path(path.clone());
        store.put(tokens(1_000_000)).expect("put");

        let status = store.status();
        assert!(status.signed_in);
        assert_eq!(status.sub.as_deref(), Some("mafio"));
        assert_eq!(status.name.as_deref(), Some("Maf"));
        assert_eq!(
            status.expires_at.as_deref(),
            Some("1970-01-12T14:46:40Z") // 1_003_600 unix seconds
        );

        let json = serde_json::to_string(&status).expect("json");
        assert!(!json.contains("ACCESS-SECRET"), "{json}");
        assert!(!json.contains("REFRESH-SECRET"), "{json}");
        assert!(!json.contains("accessToken"), "{json}");
        assert!(!json.contains("refreshToken"), "{json}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_expired_token_is_refreshed_a_minute_before_it_expires() {
        let now = 1_000_000;
        assert!(needs_refresh(now, now - 1));
        assert!(needs_refresh(now, now));
        assert!(needs_refresh(now, now + 60));
        assert!(!needs_refresh(now, now + 61));
        assert!(!needs_refresh(now, now + 3600));
    }
}
