//! File-backed JSON cache with stable md5 file names.
//!
//! - Path: `<cache_dir>/<md5(key)>.json`
//! - Payload: `{"timestamp": <unix_seconds>, "value": <json>}`
//! - Expired entries are removed on read, and a throttled sweep on write
//!   removes files nobody has touched for a long time.
//! - Writes go to a temporary file that is renamed into place, so readers
//!   never see a half-written entry.

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, io};

#[derive(Clone, Debug)]
pub struct FileJsonCache {
    dir: PathBuf,
    ttl: Duration,
    last_sweep: Arc<AtomicU64>,
}

/// How often a write may trigger a sweep of the cache directory.
const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);
/// Entries are only swept once they are older than this, even for short
/// TTLs, because caches with longer TTLs may share the directory.
const SWEEP_MIN_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl FileJsonCache {
    pub fn new(dir: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            dir: dir.into(),
            ttl,
            last_sweep: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn ensure_dir(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)
    }

    pub fn path_for(&self, key: &str) -> PathBuf {
        let digest = format!("{:x}", md5::compute(key.as_bytes()));
        self.dir.join(format!("{digest}.json"))
    }

    /// Read cached value when within TTL. Expired entries are removed.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let path = self.path_for(key);
        let raw = fs::read(&path).ok()?;
        let envelope: Value = serde_json::from_slice(&raw).ok()?;
        let ts = envelope.get("timestamp")?.as_u64()?;
        let now = now_secs();
        if now.saturating_sub(ts) < self.ttl.as_secs() {
            let value = envelope.get("value")?;
            return serde_json::from_value(value.clone()).ok();
        }

        let _ = fs::remove_file(path);
        None
    }

    /// Store value in cache envelope.
    pub fn set<T: Serialize>(&self, key: &str, value: &T) -> io::Result<()> {
        self.ensure_dir()?;
        let path = self.path_for(key);
        let envelope = serde_json::json!({
            "timestamp": now_secs(),
            "value": value,
        });
        let bytes = serde_json::to_vec(&envelope).map_err(io::Error::other)?;
        let temp = path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&temp, bytes)?;
        if let Err(error) = fs::rename(&temp, &path) {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        self.maybe_sweep();
        Ok(())
    }

    fn maybe_sweep(&self) {
        let now = now_secs();
        let last = self.last_sweep.load(Ordering::Relaxed);
        if now.saturating_sub(last) < SWEEP_INTERVAL.as_secs()
            || self
                .last_sweep
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return;
        }
        let _ = self.sweep(self.ttl.max(SWEEP_MIN_AGE));
    }

    /// Removes cache files (and stray temporary files) not modified within
    /// `older_than`. Returns how many files were removed.
    pub fn sweep(&self, older_than: Duration) -> io::Result<usize> {
        let cutoff = SystemTime::now()
            .checked_sub(older_than)
            .unwrap_or(UNIX_EPOCH);
        let temp_cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or(UNIX_EPOCH);
        let mut removed = 0;
        for entry in fs::read_dir(&self.dir)?.flatten() {
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            let limit = if extension == "json" {
                cutoff
            } else if extension.starts_with("tmp-") {
                temp_cutoff
            } else {
                continue;
            };
            let modified = entry.metadata().and_then(|meta| meta.modified());
            if modified.is_ok_and(|modified| modified < limit) && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[derive(Debug, Serialize, serde::Deserialize, PartialEq)]
    struct Sample {
        title: String,
        score: u32,
    }

    #[test]
    fn cache_path_is_deterministic_by_md5() {
        let cache = FileJsonCache::new("/tmp/stremio-core-cache-test", Duration::from_secs(10));
        let path = cache.path_for("movie:test:sample");
        let expected = format!("{:x}.json", md5::compute("movie:test:sample".as_bytes()));
        assert_eq!(
            path.file_name()
                .and_then(|value| value.to_str())
                .expect("file name"),
            expected
        );
    }

    #[test]
    fn cache_path_matches_php_layout() {
        let cache = FileJsonCache::new("/tmp/stremio-core-cache-test", Duration::from_secs(10));
        let path = cache.path_for("search:foo");
        assert_eq!(
            path.file_name().and_then(|value| value.to_str()),
            Some("8176f390f3984e8f44d851bcff380bb0.json")
        );
    }

    #[test]
    fn cache_roundtrip_and_expiration_removal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = FileJsonCache::new(tmp.path(), Duration::from_secs(0));
        let sample = Sample {
            title: "The Movie".to_string(),
            score: 42,
        };

        cache.set("immediate:expiry", &sample).expect("cache set");
        let path = cache.path_for("immediate:expiry");
        assert!(path.exists());

        let back: Option<Sample> = cache.get("immediate:expiry");
        assert!(back.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn cache_keeps_value_when_not_expired() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = FileJsonCache::new(tmp.path(), Duration::from_secs(60));

        let sample = Sample {
            title: "Keep Me".to_string(),
            score: 7,
        };
        cache.set("keep:alive", &sample).expect("cache set");

        let back: Option<Sample> = cache.get("keep:alive");
        assert_eq!(back, Some(sample));
    }

    #[test]
    fn writes_are_atomic_and_sweep_removes_old_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = FileJsonCache::new(tmp.path(), Duration::from_secs(60));
        cache.set("keep", &1u32).expect("set");
        assert_eq!(cache.get::<u32>("keep"), Some(1));

        let leftovers = fs::read_dir(tmp.path())
            .expect("read dir")
            .flatten()
            .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) != Some("json"))
            .count();
        assert_eq!(leftovers, 0);

        assert_eq!(cache.sweep(Duration::from_secs(3600)).expect("sweep"), 0);
        assert_eq!(cache.sweep(Duration::ZERO).expect("sweep"), 1);
        assert_eq!(cache.get::<u32>("keep"), None);
    }
}
