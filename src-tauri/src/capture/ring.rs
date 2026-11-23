//! The segment ring buffer: track the MPEG-TS parts written by splitmuxsink,
//! evict old ones, and hand the last N seconds to the clip maker.
//! Owned by the capture task.

use std::path::PathBuf;

use crate::error::Result;
use crate::settings::ensure_dir;

/// splitmuxsink file name pattern used for every recording session.
pub const SEGMENT_PATTERN: &str = "seg-%05u.ts";

fn is_segment(name: &str) -> bool {
    name.starts_with("seg-") && name.ends_with(".ts")
}

fn segment_index(name: &str) -> Option<u32> {
    name.strip_prefix("seg-")?.strip_suffix(".ts")?.parse().ok()
}

/// A directory of fixed-length MPEG-TS fragments. splitmuxsink writes the
/// newest one; everything with a lower index is closed and safe to read.
#[derive(Debug, Clone)]
pub struct SegmentRing {
    dir: PathBuf,
}

impl SegmentRing {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Create the directory and drop fragments left over from a previous run.
    pub fn prepare(&self) -> Result<()> {
        ensure_dir(&self.dir)?;
        self.clear();
        Ok(())
    }

    /// Every fragment on disk, ordered by index.
    pub fn list(&self) -> Vec<(u32, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut segments: Vec<(u32, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                segment_index(&name).map(|index| (index, entry.path()))
            })
            .collect();
        segments.sort_by_key(|(index, _)| *index);
        segments
    }

    pub fn latest_index(&self) -> Option<u32> {
        self.list().last().map(|(index, _)| *index)
    }

    pub fn count(&self) -> usize {
        self.list().len()
    }

    /// Seconds of footage held, counting one second per completed fragment.
    /// The fragment currently being written is not counted.
    pub fn buffered_seconds(&self) -> u32 {
        self.count().saturating_sub(1) as u32
    }

    /// The fragments covering the newest `seconds`, but never anything past
    /// `max_index` (the fragment that was open when a clip was requested).
    pub fn tail_up_to(&self, seconds: u32, max_index: u32) -> Vec<PathBuf> {
        let keep = seconds.max(1) as usize;
        let mut segments: Vec<(u32, PathBuf)> = self
            .list()
            .into_iter()
            .filter(|(index, _)| *index <= max_index)
            .collect();
        if segments.len() > keep {
            segments.drain(0..segments.len() - keep);
        }
        segments.into_iter().map(|(_, path)| path).collect()
    }

    /// Delete everything older than `keep_seconds` (plus a margin so the clip
    /// maker always has the full buffer available).
    pub fn evict(&self, keep_seconds: u32) {
        let keep = (keep_seconds as usize).saturating_add(3).max(4);
        let segments = self.list();
        if segments.len() <= keep {
            return;
        }
        for (_, path) in &segments[..segments.len() - keep] {
            if let Err(err) = std::fs::remove_file(path) {
                log::debug!("could not evict segment {}: {err}", path.display());
            }
        }
    }

    /// Remove every fragment (and stray concat lists) from the directory.
    pub fn clear(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if is_segment(&name) || name.ends_with(".concat.txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn touch(dir: &Path, index: u32) {
        std::fs::write(dir.join(format!("seg-{index:05}.ts")), b"x").unwrap();
    }

    #[test]
    fn tail_and_buffered_seconds() {
        let dir = std::env::temp_dir().join(format!("shmingus-clips-ring-{}", uuid::Uuid::new_v4()));
        let ring = SegmentRing::new(&dir);
        ring.prepare().unwrap();
        for index in 0..10 {
            touch(&dir, index);
        }
        assert_eq!(ring.count(), 10);
        assert_eq!(ring.latest_index(), Some(9));
        assert_eq!(ring.buffered_seconds(), 9);

        let tail = ring.tail_up_to(4, 7);
        let names: Vec<String> = tail
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "seg-00004.ts",
                "seg-00005.ts",
                "seg-00006.ts",
                "seg-00007.ts"
            ]
        );

        ring.evict(4);
        assert_eq!(ring.count(), 4 + 3);
        assert_eq!(ring.latest_index(), Some(9));

        ring.clear();
        assert_eq!(ring.count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
