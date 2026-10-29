//! `serve --record FILE`: keeps GSI payloads as JSON lines for later
//! analysis (real captures replace the hand-written sample, and show what the
//! client actually sends). Writes a line whenever the game state or draft
//! changes, and at least every 30 s otherwise, so a full match stays small.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::dota::gsi;

const EVERY: Duration = Duration::from_secs(30);

pub struct Recorder {
    inner: Mutex<Inner>,
}

struct Inner {
    file: File,
    last_key: Option<String>,
    last_write: Option<Instant>,
}

impl Recorder {
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Recorder {
            inner: Mutex::new(Inner {
                file,
                last_key: None,
                last_write: None,
            }),
        })
    }

    pub fn offer(&self, payload: &Value) {
        let key = json!([gsi::game_state(payload), payload.get("draft")]).to_string();
        let mut inner = self.inner.lock().expect("recorder lock");
        let due = inner.last_write.is_none_or(|t| t.elapsed() >= EVERY);
        if !due && inner.last_key.as_ref() == Some(&key) {
            return;
        }
        // The token is a credential; it has no place in a capture.
        let mut payload = payload.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("auth");
        }
        let t_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let line = json!({ "t_ms": t_ms, "payload": payload }).to_string();
        if let Err(e) = writeln!(inner.file, "{line}") {
            tracing::warn!("recording GSI payload failed: {e}");
            return;
        }
        inner.last_key = Some(key);
        inner.last_write = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_changes_only_and_strips_auth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rec.jsonl");
        let r = Recorder::open(&path).unwrap();
        let a =
            json!({"map": {"game_state": "X"}, "draft": {"activeteam": 2}, "auth": {"token": "s"}});
        let b =
            json!({"map": {"game_state": "X"}, "draft": {"activeteam": 3}, "auth": {"token": "s"}});
        r.offer(&a);
        r.offer(&a);
        r.offer(&b);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(!text.contains("token"));
    }
}
