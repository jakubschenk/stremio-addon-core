//! File-backed JSON cache with stable md5 file names.
//!
//! - Path: `<cache_dir>/<md5(key)>.json`
//! - Payload: `{"timestamp": <unix_seconds>, "value": <json>}`
//! - Expired entries are removed on read.

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, io};

#[derive(Clone, Debug)]
pub struct FileJsonCache {
    dir: PathBuf,
    ttl: Duration,
}

impl FileJsonCache {
    pub fn new(dir: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            dir: dir.into(),
            ttl,
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
        fs::write(path, bytes)
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
}
