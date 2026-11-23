//! Clip library: index, thumbnails, trim and export. Owned by the library task.
//!
//! The index is a JSON array of [`Clip`] at `config_dir()/library.json`. It is
//! loaded lazily on the first call (a missing or corrupt file simply starts an
//! empty index, which `list` then rebuilds by scanning the save folder).
//!
//! Every mutation happens under the index write lock and is persisted (temp
//! file + rename) before that lock is released, so concurrent commands can
//! neither corrupt the file nor write an older snapshot after a newer one.
//!
//! ffmpeg/ffprobe calls are synchronous; the async methods that need them move
//! the work into `tokio::task::spawn_blocking` so the runtime is never blocked
//! on a long encode or probe.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::{Mutex, RwLock};

use crate::error::{AppError, Result};
use crate::media;
use crate::model::Clip;
use crate::settings::{cache_dir, config_dir, ensure_dir};

/// Stream copies seek to the previous keyframe, so a cut can be noticeably
/// longer or shorter than requested. Past this much error we re-encode.
const TRIM_COPY_TOLERANCE_MS: u64 = 1500;
/// Thumbnail width in pixels (height follows the aspect ratio).
const THUMB_WIDTH: u32 = 640;

/// Filesystem locations the library reads and writes.
#[derive(Debug, Clone)]
struct Paths {
    index: PathBuf,
    thumbnails: PathBuf,
    exports: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            index: config_dir().join("library.json"),
            thumbnails: cache_dir().join("thumbnails"),
            exports: default_export_dir(),
        }
    }
}

fn default_export_dir() -> PathBuf {
    dirs::video_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Clipforge")
        .join("Exports")
}

pub struct Library {
    index: RwLock<Vec<Clip>>,
    loaded: AtomicBool,
    load_lock: Mutex<()>,
    persist_lock: Mutex<()>,
    paths: Paths,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            index: RwLock::new(Vec::new()),
            loaded: AtomicBool::new(false),
            load_lock: Mutex::new(()),
            persist_lock: Mutex::new(()),
            paths: Paths::default(),
        }
    }
}

impl Library {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only constructor pointing the library at isolated folders.
    #[cfg(test)]
    fn with_paths(paths: Paths) -> Self {
        Self {
            paths,
            ..Self::default()
        }
    }

    /// Rebuild the index from disk, generating missing thumbnails.
    pub async fn list(&self, save_dir: &str) -> Result<Vec<Clip>> {
        self.ensure_loaded();

        let root = PathBuf::from(save_dir);
        let scan_root = root.clone();
        let known = self.index.read().clone();
        let thumbnails = self.paths.thumbnails.clone();
        let exports = self.paths.exports.clone();

        // Probing and thumbnailing can take seconds per clip; keep it off the
        // async runtime.
        let outcome = tokio::task::spawn_blocking(move || {
            scan_and_thumb(&scan_root, &known, &thumbnails, &exports)
        })
        .await
        .map_err(|err| AppError::msg(format!("library scan failed: {err}")))?;

        let mut index = self.index.write();

        // Drop entries whose file vanished since they were added.
        index.retain(|clip| Path::new(&clip.path).exists());

        // Take the refreshed metadata/thumbnails for entries that survived.
        // Only these fields, so an `upsert` that ran while the scan was in
        // progress (e.g. a favourite toggle) is not rolled back.
        for clip in outcome.existing {
            if let Some(slot) = index.iter_mut().find(|slot| slot.id == clip.id) {
                slot.thumbnail = clip.thumbnail;
                slot.size_bytes = clip.size_bytes;
            }
        }

        // Add newly discovered files, never duplicating an id or a path.
        for clip in outcome.discovered {
            if index
                .iter()
                .any(|known| known.id == clip.id || known.path == clip.path)
            {
                continue;
            }
            index.push(clip);
        }

        sort_newest_first(&mut *index);
        let snapshot = index.clone();
        self.persist(&snapshot)?;
        Ok(snapshot)
    }

    /// Register a clip the capture engine just wrote.
    pub async fn upsert(&self, clip: Clip) -> Result<()> {
        self.ensure_loaded();
        let mut index = self.index.write();
        if let Some(slot) = index.iter_mut().find(|slot| slot.id == clip.id) {
            *slot = clip;
        } else if let Some(slot) = index.iter_mut().find(|slot| slot.path == clip.path) {
            *slot = clip;
        } else {
            index.push(clip);
        }
        sort_newest_first(&mut *index);
        let snapshot = index.clone();
        self.persist(&snapshot)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.ensure_loaded();
        let mut index = self.index.write();
        let position = index
            .iter()
            .position(|clip| clip.id == id)
            .ok_or_else(|| AppError::msg("clip not found"))?;
        let clip = index[position].clone();

        // Only ever delete the exact file the index recorded, and only if it
        // looks like a clip (absolute path to an .mp4).
        let path = PathBuf::from(&clip.path);
        if !is_clip_file(&path) {
            return Err(AppError::msg(
                "refusing to delete a path outside the library",
            ));
        }
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        // Thumbnails live in the library's own cache folder; never remove a
        // file that is not there.
        if let Some(thumbnail) = clip.thumbnail.as_deref() {
            let thumbnail = PathBuf::from(thumbnail);
            if thumbnail.starts_with(&self.paths.thumbnails) {
                let _ = std::fs::remove_file(&thumbnail);
            }
        }

        index.remove(position);
        let snapshot = index.clone();
        self.persist(&snapshot)
    }

    pub async fn rename(&self, id: &str, title: &str) -> Result<Clip> {
        self.ensure_loaded();
        let cleaned = sanitise_title(title);
        let mut index = self.index.write();
        let position = index
            .iter()
            .position(|clip| clip.id == id)
            .ok_or_else(|| AppError::msg("clip not found"))?;
        let mut clip = index[position].clone();

        let old_path = PathBuf::from(&clip.path);
        let folder = old_path
            .parent()
            .ok_or_else(|| AppError::msg("clip path has no parent folder"))?
            .to_path_buf();

        let same = folder.join(format!("{cleaned}.mp4"));
        // Renaming to the same title must not turn Foo.mp4 into Foo-2.mp4.
        let target = if same == old_path {
            old_path.clone()
        } else {
            unique_path(&folder, &cleaned, "mp4")
        };

        if target != old_path {
            if !old_path.exists() {
                return Err(AppError::msg("the clip file is missing"));
            }
            std::fs::rename(&old_path, &target)?;
            clip.path = target.to_string_lossy().to_string();
        }
        clip.title = cleaned;

        index[position] = clip.clone();
        let snapshot = index.clone();
        self.persist(&snapshot)?;
        Ok(clip)
    }

    pub async fn set_favorite(&self, id: &str, favorite: bool) -> Result<Clip> {
        self.ensure_loaded();
        let mut index = self.index.write();
        let slot = index
            .iter_mut()
            .find(|clip| clip.id == id)
            .ok_or_else(|| AppError::msg("clip not found"))?;
        slot.favorite = favorite;
        let clip = slot.clone();
        let snapshot = index.clone();
        self.persist(&snapshot)?;
        Ok(clip)
    }

    /// Write a new clip containing `start_ms..end_ms` of the original.
    pub async fn trim(&self, id: &str, start_ms: u64, end_ms: u64) -> Result<Clip> {
        self.ensure_loaded();
        let source_clip = self
            .get(id)
            .ok_or_else(|| AppError::msg("clip not found"))?;

        if end_ms <= start_ms {
            return Err(AppError::msg("the trim end must be after its start"));
        }
        if source_clip.duration_ms > 0 && start_ms >= source_clip.duration_ms {
            return Err(AppError::msg("the trim start is past the end of the clip"));
        }
        // Tolerate a UI that hands us the exact duration; never go past it.
        let end_ms = if source_clip.duration_ms > 0 {
            end_ms.min(source_clip.duration_ms)
        } else {
            end_ms
        };
        if end_ms <= start_ms {
            return Err(AppError::msg("the trim has no length"));
        }

        let source = PathBuf::from(&source_clip.path);
        if !source.exists() {
            return Err(AppError::msg("the clip file is missing"));
        }
        let folder = source
            .parent()
            .ok_or_else(|| AppError::msg("clip path has no parent folder"))?
            .to_path_buf();
        let stem = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "clip".to_string());
        let out = reserve_path(&folder, &format!("{stem}-trim"), "mp4")?;

        let new_id = uuid::Uuid::new_v4().to_string();
        let thumbnail = self.paths.thumbnails.join(format!("{new_id}.jpg"));
        let wanted_ms = end_ms - start_ms;

        let job_source = source.clone();
        let job_out = out.clone();
        let job_thumb = thumbnail.clone();
        let job = tokio::task::spawn_blocking(move || -> Result<media::MediaProbe> {
            media::remux_mp4(&job_source, &job_out, start_ms, wanted_ms)?;
            let mut probe = media::probe(&job_out)?;

            if probe.duration_ms.abs_diff(wanted_ms) > TRIM_COPY_TOLERANCE_MS {
                // Stream copying snaps to keyframes, so the copy came out too
                // far from the requested range. Re-encode this range for an
                // accurate cut (x264 + AAC keeps both audio tracks).
                reencode_segment(&job_source, &job_out, start_ms, wanted_ms)?;
                probe = media::probe(&job_out)?;
            }

            if let Err(err) = media::thumbnail(
                &job_out,
                &job_thumb,
                thumbnail_time(probe.duration_ms),
                THUMB_WIDTH,
            ) {
                log::warn!("could not generate trimmed clip thumbnail: {err}");
            }
            Ok(probe)
        });

        let probe = match job.await {
            Ok(Ok(probe)) => probe,
            Ok(Err(err)) => {
                let _ = std::fs::remove_file(&out);
                return Err(err);
            }
            Err(err) => {
                let _ = std::fs::remove_file(&out);
                return Err(AppError::msg(format!("trim failed: {err}")));
            }
        };

        let clip = Clip {
            id: new_id,
            title: format!("{} (trim)", source_clip.title),
            game: source_clip.game.clone(),
            path: out.to_string_lossy().to_string(),
            thumbnail: thumbnail
                .exists()
                .then(|| thumbnail.to_string_lossy().to_string()),
            created_at: now_rfc3339(),
            duration_ms: probe.duration_ms,
            size_bytes: media::file_size(&out),
            width: probe.width,
            height: probe.height,
            favorite: false,
        };
        self.upsert(clip.clone()).await?;
        Ok(clip)
    }

    /// Copy the clip into `dest_dir` (or the default export folder).
    pub async fn export(&self, id: &str, dest_dir: Option<&str>) -> Result<String> {
        self.ensure_loaded();
        let clip = self
            .get(id)
            .ok_or_else(|| AppError::msg("clip not found"))?;
        let source = PathBuf::from(&clip.path);
        if !source.is_file() {
            return Err(AppError::msg("the clip file is missing"));
        }

        let dest = match dest_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
            Some(dir) => std::path::absolute(dir)?,
            None => self.paths.exports.clone(),
        };
        ensure_dir(&dest)?;

        let stem = sanitise_title(&clip.title);
        let out = reserve_path(&dest, &stem, "mp4")?;

        let copy_source = source.clone();
        let copy_out = out.clone();
        let copied =
            match tokio::task::spawn_blocking(move || media::copy_clip(&copy_source, &copy_out))
                .await
            {
                Ok(result) => result,
                Err(err) => {
                    let _ = std::fs::remove_file(&out);
                    return Err(AppError::msg(format!("export failed: {err}")));
                }
            };
        if let Err(err) = copied {
            let _ = std::fs::remove_file(&out);
            return Err(err);
        }

        Ok(out.to_string_lossy().to_string())
    }

    pub fn get(&self, id: &str) -> Option<Clip> {
        self.ensure_loaded();
        self.index.read().iter().find(|clip| clip.id == id).cloned()
    }

    pub fn find_path(&self, id: &str) -> Option<PathBuf> {
        self.get(id).map(|clip| PathBuf::from(clip.path))
    }

    // -- internals ---------------------------------------------------------

    /// Read the JSON index once. A missing or unreadable/corrupt file means an
    /// empty index; the next `list` rebuilds it from the save folder.
    fn ensure_loaded(&self) {
        if self.loaded.load(Ordering::Acquire) {
            return;
        }
        let _guard = self.load_lock.lock();
        if self.loaded.load(Ordering::Acquire) {
            return;
        }

        let clips = match std::fs::read_to_string(&self.paths.index) {
            Ok(raw) => match serde_json::from_str::<Vec<Clip>>(&raw) {
                Ok(clips) => clips,
                Err(err) => {
                    log::warn!(
                        "clip library index {} is corrupt ({err}); rebuilding",
                        self.paths.index.display()
                    );
                    Vec::new()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                log::warn!(
                    "could not read clip library index {}: {err}",
                    self.paths.index.display()
                );
                Vec::new()
            }
        };

        *self.index.write() = clips;
        self.loaded.store(true, Ordering::Release);
    }

    /// Atomically replace the index file with `clips`. Callers hold the index
    /// write lock so the on-disk order matches the in-memory order.
    fn persist(&self, clips: &[Clip]) -> Result<()> {
        let _guard = self.persist_lock.lock();
        if let Some(parent) = self.paths.index.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(clips)?;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = self
            .paths
            .index
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "library.json".to_string());
        let tmp = self
            .paths
            .index
            .with_file_name(format!("{name}.{}.{seq}.tmp", std::process::id()));

        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.paths.index)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

struct ScanOutcome {
    /// Index entries that still exist, with refreshed thumbnails/metadata.
    existing: Vec<Clip>,
    /// MP4 files found on disk that were not in the index.
    discovered: Vec<Clip>,
}

/// Blocking part of `list`: verify indexed files, then find and describe new
/// ones. Never fails as a whole - an unprobeable file is skipped.
fn scan_and_thumb(
    save_dir: &Path,
    known: &[Clip],
    thumbnails: &Path,
    export_dir: &Path,
) -> ScanOutcome {
    let mut existing = Vec::new();
    let mut known_paths: HashSet<PathBuf> = HashSet::new();

    for clip in known {
        let path = PathBuf::from(&clip.path);
        if !path.is_file() {
            continue;
        }
        known_paths.insert(path.clone());

        let mut clip = clip.clone();
        let thumbnail_ok = clip
            .thumbnail
            .as_deref()
            .map(|thumb| Path::new(thumb).is_file())
            .unwrap_or(false);
        if !thumbnail_ok {
            let thumb = thumbnails.join(format!("{}.jpg", clip.id));
            match media::thumbnail(&path, &thumb, thumbnail_time(clip.duration_ms), THUMB_WIDTH) {
                Ok(()) => clip.thumbnail = Some(thumb.to_string_lossy().to_string()),
                Err(err) => log::warn!("could not thumbnail {}: {err}", path.display()),
            }
        }
        // Keep size in step with the file on disk.
        clip.size_bytes = media::file_size(&path);
        existing.push(clip);
    }

    let mut discovered = Vec::new();
    if save_dir.is_dir() {
        for path in scan_videos(save_dir, export_dir) {
            if known_paths.contains(&path) {
                continue;
            }
            let clip_path = path.to_string_lossy().to_string();
            if discovered.iter().any(|clip: &Clip| clip.path == clip_path) {
                continue;
            }

            let probe = match media::probe(&path) {
                Ok(probe) => probe,
                Err(err) => {
                    log::warn!("skipping unreadable clip {}: {err}", path.display());
                    continue;
                }
            };

            let id = uuid::Uuid::new_v4().to_string();
            let thumb = thumbnails.join(format!("{id}.jpg"));
            let thumbnail = match media::thumbnail(
                &path,
                &thumb,
                thumbnail_time(probe.duration_ms),
                THUMB_WIDTH,
            ) {
                Ok(()) => Some(thumb.to_string_lossy().to_string()),
                Err(err) => {
                    log::warn!("could not thumbnail {}: {err}", path.display());
                    None
                }
            };

            discovered.push(Clip {
                id,
                title: title_from_path(&path),
                game: game_from_path(&path, save_dir),
                path: clip_path,
                thumbnail,
                created_at: modified_rfc3339(&path),
                duration_ms: probe.duration_ms,
                size_bytes: probe.size_bytes,
                width: probe.width,
                height: probe.height,
                favorite: false,
            });
        }
    }

    ScanOutcome {
        existing,
        discovered,
    }
}

/// All `.mp4` files below `root`, without following symlinks. The export
/// folder is skipped: copied clips there would otherwise be indexed twice.
fn scan_videos(root: &Path, export_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(dir) = pending.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                log::warn!("could not read {}: {err}", dir.display());
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };

            if file_type.is_dir() {
                if path == export_dir || is_default_exports_child(&path, root) {
                    continue;
                }
                pending.push(path);
            } else if file_type.is_file() && has_mp4_extension(&path) && !is_hidden(&path) {
                found.push(path);
            }
        }
    }

    found.sort();
    found
}

fn is_default_exports_child(path: &Path, save_dir: &Path) -> bool {
    path.parent() == Some(save_dir)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("exports"))
            .unwrap_or(false)
}

fn has_mp4_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("mp4"))
        .unwrap_or(false)
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

/// Human title from a file name: drop the extension, turn `-`/`_` into spaces.
fn title_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    let spaced = stem.replace(['-', '_'], " ");
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "Clip".to_string()
    } else {
        collapsed
    }
}

/// A clip in a game sub-folder is tagged with that folder's name; clips at the
/// save-dir root are untagged.
fn game_from_path(path: &Path, save_dir: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent == save_dir {
        return None;
    }
    parent
        .file_name()
        .map(|name| name.to_string_lossy().trim().to_string())
        .filter(|name| !name.is_empty())
}

fn thumbnail_time(duration_ms: u64) -> u64 {
    if duration_ms < 1000 {
        duration_ms / 5
    } else {
        1000
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Make a title safe for use as a file name and sane to display.
fn sanitise_title(input: &str) -> String {
    let mut cleaned = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_control() || matches!(ch, '/' | '\\' | ':') {
            cleaned.push(' ');
        } else {
            cleaned.push(ch);
        }
    }
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let title: String = collapsed.chars().take(80).collect();
    let title = title.trim().trim_end_matches('.').trim();
    if title.is_empty() {
        "Clip".to_string()
    } else {
        title.to_string()
    }
}

/// `<dir>/<stem>.<ext>`, then `-2`, `-3`… until the name is free.
fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}.{ext}"));
    let mut suffix = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-{suffix}.{ext}"));
        suffix += 1;
    }
    candidate
}

/// Like [`unique_path`], but claims the name by creating an empty placeholder,
/// so two concurrent trims/exports cannot pick the same target. ffmpeg and
/// `media::copy_clip` overwrite the placeholder.
fn reserve_path(dir: &Path, stem: &str, ext: &str) -> Result<PathBuf> {
    let mut suffix = 1;
    loop {
        let name = if suffix == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{suffix}.{ext}")
        };
        let candidate = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => suffix += 1,
            Err(err) => return Err(err.into()),
        }
    }
}

fn is_clip_file(path: &Path) -> bool {
    path.file_name().is_some() && has_mp4_extension(path)
}

fn sort_newest_first(clips: &mut Vec<Clip>) {
    clips.sort_by(|a, b| {
        match (
            parse_timestamp(&a.created_at),
            parse_timestamp(&b.created_at),
        ) {
            (Some(left), Some(right)) => right.cmp(&left),
            _ => b.created_at.cmp(&a.created_at),
        }
    });
}

fn parse_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn modified_rfc3339(path: &Path) -> String {
    match std::fs::metadata(path).and_then(|meta| meta.modified()) {
        Ok(time) => chrono::DateTime::<chrono::Utc>::from(time)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        Err(_) => now_rfc3339(),
    }
}

/// Accurate re-encode of `duration_ms` starting at `start_ms`, keeping every
/// stream (`-map 0`) so both audio tracks survive. Used as the trim fallback
/// when a stream copy lands too far from the requested range.
fn reencode_segment(input: &Path, out: &Path, start_ms: u64, duration_ms: u64) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let start = format!("{:.3}", start_ms as f64 / 1000.0);
    let duration = format!("{:.3}", duration_ms as f64 / 1000.0);
    media::run(
        "ffmpeg",
        &[
            "-y".as_ref(),
            "-ss".as_ref(),
            start.as_ref(),
            "-t".as_ref(),
            duration.as_ref(),
            "-i".as_ref(),
            input.as_os_str(),
            "-map".as_ref(),
            "0".as_ref(),
            "-c:v".as_ref(),
            "libx264".as_ref(),
            "-preset".as_ref(),
            "veryfast".as_ref(),
            "-crf".as_ref(),
            "20".as_ref(),
            "-c:a".as_ref(),
            "aac".as_ref(),
            "-b:a".as_ref(),
            "192k".as_ref(),
            "-movflags".as_ref(),
            "+faststart".as_ref(),
            "-avoid_negative_ts".as_ref(),
            "make_zero".as_ref(),
            out.as_os_str(),
        ],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("clipforge-library-{tag}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_library(root: &Path) -> Library {
        Library::with_paths(Paths {
            index: root.join("config").join("library.json"),
            thumbnails: root.join("cache").join("thumbnails"),
            exports: root.join("exports"),
        })
    }

    fn sample_clip(id: &str, root: &Path) -> Clip {
        Clip {
            id: id.to_string(),
            title: "A sample clip".to_string(),
            game: Some("Test Game".to_string()),
            path: root.join(format!("{id}.mp4")).to_string_lossy().to_string(),
            thumbnail: None,
            created_at: "2026-01-02T03:04:05.000Z".to_string(),
            duration_ms: 12_000,
            size_bytes: 1_234,
            width: 1280,
            height: 720,
            favorite: false,
        }
    }

    fn ffmpeg_ready() -> bool {
        media::ffmpeg_available() && media::ffprobe_available()
    }

    /// Small H.264/AAC MP4 with two audio tracks, 1s keyframe interval, so a
    /// stream copy at whole seconds is accurate.
    fn make_fixture(dir: &Path, name: &str, seconds: u32) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create fixture dir");
        let out = dir.join(name);
        let length = seconds.to_string();
        let video = format!("testsrc=size=320x180:rate=30:duration={length}");
        let audio_a = format!("sine=frequency=440:duration={length}");
        let audio_b = format!("sine=frequency=880:duration={length}");

        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &video,
                "-f",
                "lavfi",
                "-i",
                &audio_a,
                "-f",
                "lavfi",
                "-i",
                &audio_b,
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-map",
                "2:a",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                "30",
                "-keyint_min",
                "30",
                "-sc_threshold",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "64k",
                "-shortest",
            ])
            .arg(&out)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "could not generate the test clip");
        out
    }

    // -- pure helpers ------------------------------------------------------

    #[test]
    fn titles_are_sanitised() {
        assert_eq!(sanitise_title("  A/B\\C: D  "), "A B C D");
        assert_eq!(sanitise_title("line\nbreak\there"), "line break here");
        assert_eq!(sanitise_title("   "), "Clip");
        assert_eq!(sanitise_title("trailing dots..."), "trailing dots");
        assert_eq!(sanitise_title(&"x".repeat(120)).chars().count(), 80);
        assert_eq!(
            title_from_path(Path::new("/clips/my-cool_clip.mp4")),
            "my cool clip"
        );
    }

    #[test]
    fn unique_paths_get_numeric_suffixes() {
        let tmp = TempDir::new("unique");
        let dir = tmp.path();
        assert_eq!(unique_path(dir, "Clip", "mp4"), dir.join("Clip.mp4"));
        std::fs::write(dir.join("Clip.mp4"), b"x").unwrap();
        assert_eq!(unique_path(dir, "Clip", "mp4"), dir.join("Clip-2.mp4"));
        std::fs::write(dir.join("Clip-2.mp4"), b"x").unwrap();
        assert_eq!(unique_path(dir, "Clip", "mp4"), dir.join("Clip-3.mp4"));
    }

    // -- index persistence -------------------------------------------------

    #[tokio::test]
    async fn index_round_trips_through_disk() {
        let tmp = TempDir::new("index");
        let lib = test_library(tmp.path());
        let clip = sample_clip("clip-1", tmp.path());
        lib.upsert(clip.clone()).await.unwrap();
        lib.set_favorite("clip-1", true).await.unwrap();
        drop(lib);

        let reopened = test_library(tmp.path());
        let saved = reopened.get("clip-1").expect("index survived a restart");
        assert_eq!(saved.title, clip.title);
        assert_eq!(saved.game, clip.game);
        assert_eq!(saved.path, clip.path);
        assert!(saved.favorite);
        assert_eq!(saved.duration_ms, clip.duration_ms);
    }

    #[tokio::test]
    async fn corrupt_index_starts_empty_and_is_rebuilt() {
        let tmp = TempDir::new("corrupt");
        let index = tmp.path().join("config").join("library.json");
        std::fs::create_dir_all(index.parent().unwrap()).unwrap();
        std::fs::write(&index, "{ this is not json").unwrap();

        let save = tmp.path().join("clips");
        std::fs::create_dir_all(&save).unwrap();
        let lib = test_library(tmp.path());
        let clips = lib.list(&save.to_string_lossy()).await.unwrap();
        assert!(clips.is_empty());

        // The repaired file parses now.
        let raw = std::fs::read_to_string(&index).unwrap();
        let parsed: Vec<Clip> = serde_json::from_str(&raw).expect("valid index after rebuild");
        assert!(parsed.is_empty());
    }

    #[tokio::test]
    async fn upsert_replaces_by_id_and_lists_newest_first() {
        let tmp = TempDir::new("upsert");
        let lib = test_library(tmp.path());

        let mut older = sample_clip("older", tmp.path());
        older.created_at = "2026-01-01T00:00:00.000Z".to_string();
        let mut newer = sample_clip("newer", tmp.path());
        newer.created_at = "2026-02-01T00:00:00.000Z".to_string();
        // `list` drops index entries whose file vanished, so give them files.
        std::fs::write(&older.path, b"x").unwrap();
        std::fs::write(&newer.path, b"x").unwrap();
        lib.upsert(older).await.unwrap();
        lib.upsert(newer.clone()).await.unwrap();

        let mut changed = newer;
        changed.title = "Renamed elsewhere".to_string();
        lib.upsert(changed).await.unwrap();

        let save = tmp.path().join("empty");
        std::fs::create_dir_all(&save).unwrap();
        let clips = lib.list(&save.to_string_lossy()).await.unwrap();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].id, "newer");
        assert_eq!(clips[0].title, "Renamed elsewhere");
        assert_eq!(clips[1].id, "older");
    }

    // -- discovery ---------------------------------------------------------

    #[tokio::test]
    async fn scan_discovers_clips_with_metadata_and_thumbnails() {
        if !ffmpeg_ready() {
            eprintln!("skipping: ffmpeg/ffprobe unavailable");
            return;
        }

        let tmp = TempDir::new("scan");
        let save = tmp.path().join("clips");
        make_fixture(&save.join("Test Game"), "clip-one.mp4", 6);

        let lib = test_library(tmp.path());
        let clips = lib.list(&save.to_string_lossy()).await.unwrap();
        assert_eq!(clips.len(), 1);

        let clip = &clips[0];
        assert_eq!(clip.title, "clip one");
        assert_eq!(clip.game.as_deref(), Some("Test Game"));
        assert_eq!((clip.width, clip.height), (320, 180));
        assert!(
            clip.duration_ms.abs_diff(6000) < 500,
            "duration {}",
            clip.duration_ms
        );
        assert!(clip.size_bytes > 0);
        assert!(clip.created_at.starts_with("20"));
        assert!(!clip.favorite);

        let thumbnail = PathBuf::from(clip.thumbnail.as_ref().expect("thumbnail"));
        assert!(
            thumbnail.is_file(),
            "thumbnail missing at {}",
            thumbnail.display()
        );

        // A second instance reuses the persisted index rather than re-adding.
        let reopened = test_library(tmp.path());
        assert!(reopened.get(&clip.id).is_some());
        let again = reopened.list(&save.to_string_lossy()).await.unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].id, clip.id);
    }

    #[tokio::test]
    async fn vanished_files_are_dropped_from_the_index() {
        if !ffmpeg_ready() {
            eprintln!("skipping: ffmpeg/ffprobe unavailable");
            return;
        }

        let tmp = TempDir::new("vanish");
        let save = tmp.path().join("clips");
        let fixture = make_fixture(&save, "gone.mp4", 2);

        let lib = test_library(tmp.path());
        assert_eq!(lib.list(&save.to_string_lossy()).await.unwrap().len(), 1);

        std::fs::remove_file(&fixture).unwrap();
        assert!(lib.list(&save.to_string_lossy()).await.unwrap().is_empty());
    }

    // -- trim / rename / export / delete -----------------------------------

    #[tokio::test]
    async fn end_to_end_trim_rename_export_delete() {
        if !ffmpeg_ready() {
            eprintln!("skipping: ffmpeg/ffprobe unavailable");
            return;
        }

        let tmp = TempDir::new("e2e");
        let save = tmp.path().join("clips");
        make_fixture(&save, "clip-one.mp4", 6);

        let lib = test_library(tmp.path());
        let original = lib.list(&save.to_string_lossy()).await.unwrap().remove(0);

        // Trim 1s..3s: a new clip, original untouched, both audio tracks kept.
        let trimmed = lib.trim(&original.id, 1000, 3000).await.unwrap();
        assert_ne!(trimmed.id, original.id);
        assert_eq!(trimmed.title, "clip one (trim)");
        assert_eq!(trimmed.game, original.game);
        assert!(trimmed.path.ends_with("-trim.mp4"), "{}", trimmed.path);
        assert!(Path::new(&trimmed.path).is_file());
        assert!(
            Path::new(&original.path).is_file(),
            "the original must survive"
        );
        assert!(Path::new(trimmed.thumbnail.as_ref().expect("trim thumbnail")).is_file());

        let probe = media::probe(Path::new(&trimmed.path)).unwrap();
        assert_eq!(probe.audio_streams, 2, "both audio tracks must survive");
        assert!(
            probe.duration_ms.abs_diff(2000) <= 100,
            "trimmed duration {} is not 2s",
            probe.duration_ms
        );

        // Export copies into the requested folder.
        let export_dir = tmp.path().join("out");
        let exported = lib
            .export(&trimmed.id, Some(&export_dir.to_string_lossy()))
            .await
            .unwrap();
        let exported = PathBuf::from(exported);
        assert!(exported.is_file());
        assert!(exported.starts_with(&export_dir));
        assert_eq!(
            media::file_size(&exported),
            media::file_size(Path::new(&trimmed.path))
        );

        // Rename moves the file and sanitises the title.
        let old_path = PathBuf::from(&trimmed.path);
        let renamed = lib.rename(&trimmed.id, "Best/Win: run").await.unwrap();
        assert_eq!(renamed.title, "Best Win run");
        assert!(!old_path.exists(), "the old file name must be gone");
        let new_path = PathBuf::from(&renamed.path);
        assert!(new_path.is_file());
        assert_eq!(new_path.file_name().unwrap(), "Best Win run.mp4");
        assert_eq!(lib.get(&trimmed.id).unwrap().path, renamed.path);

        // Delete removes the file, its thumbnail and the entry.
        let original_thumbnail = PathBuf::from(original.thumbnail.clone().unwrap());
        let original_path = PathBuf::from(&original.path);
        lib.delete(&original.id).await.unwrap();
        assert!(!original_path.exists());
        assert!(!original_thumbnail.exists());
        assert!(lib.get(&original.id).is_none());

        let remaining = lib.list(&save.to_string_lossy()).await.unwrap();
        assert!(remaining.iter().all(|clip| clip.id != original.id));
        assert!(remaining.iter().any(|clip| clip.id == trimmed.id));
    }

    #[tokio::test]
    async fn trim_rejects_invalid_ranges() {
        if !ffmpeg_ready() {
            eprintln!("skipping: ffmpeg/ffprobe unavailable");
            return;
        }

        let tmp = TempDir::new("trim-invalid");
        let save = tmp.path().join("clips");
        make_fixture(&save, "clip.mp4", 4);
        let lib = test_library(tmp.path());
        let clip = lib.list(&save.to_string_lossy()).await.unwrap().remove(0);

        assert!(lib.trim(&clip.id, 3000, 3000).await.is_err());
        assert!(lib.trim(&clip.id, 5000, 6000).await.is_err());
        assert!(lib.delete("does-not-exist").await.is_err());
    }
}
