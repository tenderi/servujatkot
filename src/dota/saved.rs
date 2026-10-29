//! Dota's own saved settings, and our snapshot of them.
//!
//! Dota writes every convar and console bind that differs from the default to
//! `<Steam>/userdata/<account>/570/local/cfg/` (`user_convars_0_slot0.vcfg`,
//! `machine_convars.vcfg`, `user_keys_0_slot0.vcfg`) when it exits, so
//! anything the cast cfg sets outlives the session. Before the first install
//! we record the current value of every convar and key we touch; the play cfg
//! restores exactly that, and uninstall checks it has been restored.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use super::cfg::CastingConfig;

const CONVAR_FILES: &[&str] = &["user_convars_0_slot0.vcfg", "machine_convars.vcfg"];
const KEYS_FILE: &str = "user_keys_0_slot0.vcfg";

/// Every `<root>/userdata/<account>/570/local/cfg` under the given Steam roots,
/// most recently written first.
pub fn find_saved_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for root in roots {
        let Ok(accounts) = std::fs::read_dir(root.join("userdata")) else {
            continue;
        };
        for acc in accounts.flatten() {
            let dir = acc.path().join("570/local/cfg");
            let Ok(meta) = std::fs::metadata(dir.join(CONVAR_FILES[0])) else {
                continue;
            };
            let dir = dir.canonicalize().unwrap_or(dir);
            if !found.iter().any(|(_, d)| *d == dir) {
                found.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), dir));
            }
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, d)| d).collect()
}

/// What Dota currently has saved. Missing means "at the default".
#[derive(Debug, Default)]
pub struct Saved {
    pub convars: HashMap<String, String>,
    /// Keyed by upper-cased key name.
    pub binds: HashMap<String, String>,
}

static KV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*"([^"]+)"\s+"([^"]*)"\s*$"#).unwrap());

fn kv_lines(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines()
        .filter_map(|l| KV.captures(l))
        .map(|c| (c[1].to_string(), c[2].to_string()))
}

impl Saved {
    pub fn load(dir: &Path) -> Result<Self> {
        let mut saved = Saved::default();
        for f in CONVAR_FILES {
            if let Some(text) = read_opt(&dir.join(f))? {
                saved.convars.extend(kv_lines(&text));
            }
        }
        if let Some(text) = read_opt(&dir.join(KEYS_FILE))? {
            saved.binds.extend(
                kv_lines(&text)
                    .filter(|(_, v)| v != "<unbound>")
                    .map(|(k, v)| (k.to_uppercase(), v)),
            );
        }
        Ok(saved)
    }

    fn bind(&self, key: &str) -> Option<&String> {
        self.binds.get(&key.to_uppercase())
    }
}

fn read_opt(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// `true`/`false` as the console's `1`/`0`, numbers in one canonical form,
/// so `3000`, `3000.0` and `"3000.000000"` compare equal.
pub fn norm(v: &str) -> String {
    match v.trim().to_lowercase().as_str() {
        "true" => "1".into(),
        "false" => "0".into(),
        s => s
            .parse::<f64>()
            .map_or_else(|_| s.to_string(), |f| f.to_string()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConvarEntry {
    /// What Dota had saved; `None` = it was at the default.
    pub saved: Option<String>,
    /// What the play cfg sets: `saved`, else casting.toml's `play` fallback
    /// (we can't read a default). `None` = cannot be restored.
    pub restore: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// The userdata directory the values were read from.
    pub source: Option<PathBuf>,
    pub convars: BTreeMap<String, ConvarEntry>,
    /// Upper-cased key -> command; `None` = the key was unbound.
    pub binds: BTreeMap<String, Option<String>>,
    /// The cast/play switch keys. The play cfg can't restore these (it runs
    /// from one of them), so uninstall prints the commands instead.
    #[serde(default)]
    pub switch_keys: Vec<String>,
}

impl Snapshot {
    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("parsing snapshot")
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("snapshot serializes") + "\n"
    }

    /// Records every convar and key in `config` (plus `extra_keys`) that the
    /// snapshot doesn't have yet. Existing entries are never overwritten: by
    /// now Dota may have saved casting values over the originals.
    /// Returns the convars that have no restore value.
    pub fn extend(
        &mut self,
        config: &CastingConfig,
        extra_keys: &[&str],
        saved: &Saved,
    ) -> Vec<String> {
        for s in config.settings.iter().filter(|s| s.enabled) {
            self.convars.entry(s.key.clone()).or_insert_with(|| {
                let was = saved.convars.get(&s.key).map(|v| norm(v));
                ConvarEntry {
                    restore: was.clone().or_else(|| s.play.as_deref().map(norm)),
                    saved: was,
                }
            });
        }
        let keys = config
            .binds
            .iter()
            .map(|b| b.key.as_str())
            .chain(config.spot_keys())
            .chain(extra_keys.iter().copied());
        for key in keys {
            self.binds
                .entry(key.to_uppercase())
                .or_insert_with(|| saved.bind(key).cloned());
        }
        self.convars
            .iter()
            .filter(|(_, e)| e.restore.is_none())
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Ways in which Dota's saved state differs from the snapshot, as
    /// human-readable lines. Empty = fully restored.
    pub fn differences(&self, now: &Saved, skip_binds: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        for (key, e) in &self.convars {
            let Some(want) = &e.restore else { continue };
            let have = now.convars.get(key).map(|v| norm(v));
            // Unsaved means default; if the original was unsaved too, that matches.
            let ok = have.as_ref() == Some(want) || (have.is_none() && e.saved.is_none());
            if !ok {
                out.push(format!(
                    "{key} is {}, was {want}",
                    have.as_deref().unwrap_or("default")
                ));
            }
        }
        for (key, want) in &self.binds {
            if skip_binds.iter().any(|k| k.eq_ignore_ascii_case(key)) {
                continue;
            }
            let have = now.bind(key);
            if have != want.as_ref() {
                out.push(format!(
                    "{key} is bound to {}, was {}",
                    have.map_or("nothing".into(), |c| format!("`{c}`")),
                    want.as_ref().map_or("nothing".into(), |c| format!("`{c}`")),
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONVARS: &str = "\"config\"\n{\n\t\"convars\"\n\t{\n\t\t\"dota_camera_speed\"\t\t\"6000\"\n\t\t\"dota_camera_edgemove\"\t\t\"true\"\n\t}\n}\n";
    const KEYS: &str = "\"config\"\n{\n\t\"bindings\"\n\t{\n\t\t\"SPACE\"\t\t\"+camhold\"\n\t\t\"F11\"\t\t\"exec pimpcasting.cfg\"\n\t\t\"MOUSE5\"\t\t\"<unbound>\"\n\t}\n}\n";

    fn saved_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("user_convars_0_slot0.vcfg"), CONVARS).unwrap();
        std::fs::write(d.path().join(KEYS_FILE), KEYS).unwrap();
        d
    }

    fn config() -> CastingConfig {
        CastingConfig::parse(
            r#"
            [[setting]]
            key = "dota_camera_speed"
            cast = "3000"
            play = "5000"
            [[setting]]
            key = "dota_camera_edgemove"
            cast = "0"
            [[setting]]
            key = "dota_camera_accelerate"
            cast = "5"
            play = "50"
            [[setting]]
            key = "dota_spec_talentchanges_show"
            cast = "0"
            [[bind]]
            key = "space"
            cast = "+dota_camera_follow"
            [[bind]]
            key = "MOUSE4"
            cast = "dota_toggle_assisted_camera_operator"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn reads_saved_values_and_normalises() {
        let d = saved_dir();
        let s = Saved::load(d.path()).unwrap();
        assert_eq!(s.convars["dota_camera_edgemove"], "true");
        assert_eq!(s.bind("space").unwrap(), "+camhold");
        assert!(s.bind("MOUSE5").is_none(), "<unbound> is unbound");
        assert_eq!(norm("true"), "1");
        assert_eq!(norm("3000.000000"), "3000");
        assert_eq!(norm("0.0025"), "0.0025");
    }

    #[test]
    fn snapshot_prefers_saved_then_toml_and_never_overwrites() {
        let d = saved_dir();
        let saved = Saved::load(d.path()).unwrap();
        let mut snap = Snapshot::default();
        let missing = snap.extend(&config(), &["F11"], &saved);

        assert_eq!(
            snap.convars["dota_camera_speed"].restore.as_deref(),
            Some("6000"),
            "saved beats toml"
        );
        assert_eq!(
            snap.convars["dota_camera_edgemove"].restore.as_deref(),
            Some("1")
        );
        assert_eq!(
            snap.convars["dota_camera_accelerate"].restore.as_deref(),
            Some("50"),
            "toml fallback"
        );
        assert_eq!(missing, vec!["dota_spec_talentchanges_show".to_string()]);
        assert_eq!(snap.binds["SPACE"].as_deref(), Some("+camhold"));
        assert_eq!(snap.binds["MOUSE4"], None);
        assert_eq!(snap.binds["F11"].as_deref(), Some("exec pimpcasting.cfg"));

        // Dota has since saved casting values; extending again keeps the originals.
        let mut casting = Saved::default();
        casting
            .convars
            .insert("dota_camera_speed".into(), "3000".into());
        snap.extend(&config(), &[], &casting);
        assert_eq!(
            snap.convars["dota_camera_speed"].restore.as_deref(),
            Some("6000")
        );

        let back = Snapshot::parse(&snap.to_json()).unwrap();
        assert_eq!(back.convars, snap.convars);
    }

    #[test]
    fn differences_spot_unrestored_state() {
        let d = saved_dir();
        let before = Saved::load(d.path()).unwrap();
        let mut snap = Snapshot::default();
        snap.extend(&config(), &[], &before);
        assert!(snap.differences(&before, &[]).is_empty());

        let mut after = Saved::load(d.path()).unwrap();
        after
            .convars
            .insert("dota_camera_speed".into(), "3000".into());
        after.binds.insert(
            "MOUSE4".into(),
            "dota_toggle_assisted_camera_operator".into(),
        );
        after
            .binds
            .insert("F11".into(), "exec servujatkot_cast".into());
        let diffs = snap.differences(&after, &["f11"]);
        assert_eq!(diffs.len(), 2, "{diffs:?}");
        assert!(diffs[0].contains("dota_camera_speed is 3000, was 6000"));
        assert!(diffs[1].contains("MOUSE4"));
    }
}
