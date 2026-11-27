//! Persistent record of the shares this app uploaded.
//!
//! The file lives at `~/.config/shmingus-clips/shares.json` and is keyed by clip id.
//! Each record keeps the public [`ShareInfo`] plus the secret the server issued
//! for deleting the share and the server it lives on. A missing or corrupt file
//! is treated as an empty store: sharing still works, and the next upload
//! rewrites the file.
//!
//! Writes go to a temp file followed by a rename, so a crash mid-write cannot
//! leave a truncated index behind.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::model::ShareInfo;
use crate::settings::config_dir;

/// Everything needed to manage a share the app created.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareRecord {
    pub share: ShareInfo,
    /// Never sent to the frontend; only used to delete the share.
    pub delete_token: String,
    /// The server the share was uploaded to, so a later settings change does
    /// not strand the record.
    pub base_url: String,
}

pub struct ShareStore {
    path: PathBuf,
    inner: Mutex<HashMap<String, ShareRecord>>,
}

impl ShareStore {
    /// The live store, backed by `~/.config/shmingus-clips/shares.json`.
    pub fn load() -> Arc<Self> {
        Arc::new(Self::from_path(default_path()))
    }

    /// Build a store backed by `path`, reading whatever is already there.
    /// Path injection exists so tests can round-trip a temp file.
    pub fn from_path(path: PathBuf) -> Self {
        let records = match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<HashMap<String, ShareRecord>>(&raw) {
                Ok(records) => records,
                Err(err) => {
                    log::warn!(
                        "{} is corrupt; starting with no remembered shares: {err}",
                        path.display()
                    );
                    HashMap::new()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(err) => {
                log::warn!("could not read {}: {err}", path.display());
                HashMap::new()
            }
        };

        // The file holds delete tokens; keep it private even if an older build
        // created it with default permissions.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        Self {
            path,
            inner: Mutex::new(records),
        }
    }

    /// The public share for a clip, if it was uploaded.
    pub fn get(&self, clip_id: &str) -> Option<ShareInfo> {
        self.inner
            .lock()
            .get(clip_id)
            .map(|record| record.share.clone())
    }

    /// The full record, including the delete token, for internal use.
    pub fn record(&self, clip_id: &str) -> Option<ShareRecord> {
        self.inner.lock().get(clip_id).cloned()
    }

    /// Every remembered share, clip id -> share.
    pub fn all(&self) -> HashMap<String, ShareInfo> {
        self.inner
            .lock()
            .iter()
            .map(|(clip_id, record)| (clip_id.clone(), record.share.clone()))
            .collect()
    }

    /// Remember (or replace) the share of `clip_id`, persisting it first.
    pub fn put(&self, clip_id: &str, record: ShareRecord) -> Result<()> {
        let mut inner = self.inner.lock();
        let previous = inner.insert(clip_id.to_string(), record);

        match self.persist(&inner) {
            Ok(()) => Ok(()),
            Err(err) => {
                // Keep memory and disk in agreement: undo the in-memory change
                // the failed write was meant to record.
                match previous {
                    Some(previous) => {
                        inner.insert(clip_id.to_string(), previous);
                    }
                    None => {
                        inner.remove(clip_id);
                    }
                }
                Err(err)
            }
        }
    }

    /// Forget the share of `clip_id`; a no-op when there is none.
    pub fn remove(&self, clip_id: &str) -> Result<()> {
        let mut inner = self.inner.lock();
        let Some(previous) = inner.remove(clip_id) else {
            return Ok(());
        };

        match self.persist(&inner) {
            Ok(()) => Ok(()),
            Err(err) => {
                inner.insert(clip_id.to_string(), previous);
                Err(err)
            }
        }
    }

    /// Write the whole index through a temp file so readers never see a
    /// half-written JSON document.
    fn persist(&self, records: &HashMap<String, ShareRecord>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(records)?;
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600));
        }
        if let Err(err) = std::fs::rename(&temp, &self.path) {
            let _ = std::fs::remove_file(&temp);
            return Err(AppError::from(err));
        }
        Ok(())
    }
}

fn default_path() -> PathBuf {
    config_dir().join("shares.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "shmingus-clips-shares-{}-{label}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }

    fn record(id: &str) -> ShareRecord {
        ShareRecord {
            share: ShareInfo {
                id: id.into(),
                url: format!("https://clips.example.com/s/{id}"),
                embed_url: format!("https://clips.example.com/embed/{id}"),
                video_url: format!("https://clips.example.com/v/{id}.mp4"),
                created_at: "2026-11-22T12:00:00Z".into(),
            },
            delete_token: format!("secret-{id}"),
            base_url: "https://clips.example.com".into(),
        }
    }

    #[test]
    fn records_round_trip_through_the_file() {
        let path = temp_path("round-trip");
        let store = ShareStore::from_path(path.clone());
        assert!(store.all().is_empty());

        store.put("clip-a", record("one")).expect("put a");
        store.put("clip-b", record("two")).expect("put b");
        assert_eq!(store.get("clip-a").expect("get a").id, "one");
        assert_eq!(
            store.record("clip-b").expect("record b").delete_token,
            "secret-two"
        );

        let reopened = ShareStore::from_path(path.clone());
        let all = reopened.all();
        assert_eq!(all.len(), 2);
        assert_eq!(all["clip-a"].url, "https://clips.example.com/s/one");
        assert_eq!(
            reopened.record("clip-b").expect("record b").base_url,
            "https://clips.example.com"
        );

        reopened.remove("clip-a").expect("remove a");
        assert!(reopened.get("clip-a").is_none());

        let reopened_again = ShareStore::from_path(path.clone());
        assert!(reopened_again.get("clip-a").is_none());
        assert!(reopened_again.get("clip-b").is_some());
        assert_eq!(
            reopened_again
                .record("clip-b")
                .expect("record b")
                .delete_token,
            "secret-two"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_file_starts_empty_and_is_only_created_by_a_write() {
        let path = temp_path("missing");
        let store = ShareStore::from_path(path.clone());
        assert!(store.all().is_empty());
        assert!(!path.exists());

        store.put("clip-a", record("one")).expect("put");
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_corrupt_file_is_treated_as_empty_and_repaired_on_write() {
        let path = temp_path("corrupt");
        std::fs::write(&path, b"this is not json").expect("write corrupt file");

        let store = ShareStore::from_path(path.clone());
        assert!(store.all().is_empty());

        store.put("clip-a", record("one")).expect("put");
        let reopened = ShareStore::from_path(path.clone());
        assert_eq!(reopened.get("clip-a").expect("get a").id, "one");

        let _ = std::fs::remove_file(&path);
    }
}
