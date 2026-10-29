//! Finding the Dota 2 install (Linux Steam layouts) and writing our files
//! into it: the two console cfgs, the GSI cfg, and a marked block in
//! `autoexec.cfg` that binds the switch keys.
//!
//! Work is split into a [`Plan`] (pure, testable, printable for `--dry-run`)
//! and [`Plan::apply`].

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;

use super::cfg::{self, CastingConfig};
use super::gsi;
use super::saved::{Saved, Snapshot};

const BEGIN: &str = "// >>> servujatkot";
const END: &str = "// <<< servujatkot";
const BACKUP: &str = "autoexec.cfg.servujatkot-backup";
const SNAPSHOT: &str = "servujatkot_snapshot.json";

/// Steam roots to look in, most common first: native, the `~/.steam`
/// symlinks, Flatpak and Snap.
pub fn steam_roots(home: &Path) -> Vec<PathBuf> {
    [
        ".local/share/Steam",
        ".steam/steam",
        ".steam/root",
        ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        "snap/steam/common/.local/share/Steam",
    ]
    .iter()
    .map(|p| home.join(p))
    .collect()
}

static VDF_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""path"\s+"((?:[^"\\]|\\.)*)""#).unwrap());

/// Library folders listed in `steamapps/libraryfolders.vdf`, plus the root
/// itself (older files don't list it).
fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![root.to_path_buf()];
    if let Ok(vdf) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        libs.extend(
            VDF_PATH
                .captures_iter(&vdf)
                .map(|c| PathBuf::from(c[1].replace("\\\\", "\\"))),
        );
    }
    libs
}

pub fn is_dota_dir(dir: &Path) -> bool {
    dir.join("game/dota/cfg").is_dir()
}

/// Every distinct Dota 2 install reachable from the Steam roots under `home`.
pub fn find_dota_dirs(home: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for root in steam_roots(home) {
        for lib in libraries(&root) {
            let dir = lib.join("steamapps/common/dota 2 beta");
            if !is_dota_dir(&dir) {
                continue;
            }
            // The ~/.steam symlinks point at the same install.
            let dir = dir.canonicalize().unwrap_or(dir);
            if !found.contains(&dir) {
                found.push(dir);
            }
        }
    }
    found
}

/// Candidate `dota 2 beta` directories: the one given (validated), or every
/// install found in the Steam libraries. Never empty. The caller confirms the
/// choice with the user; nothing here assumes a location is right.
pub fn dota_dir_candidates(given: Option<PathBuf>) -> Result<Vec<PathBuf>> {
    if let Some(dir) = given {
        if !is_dota_dir(&dir) {
            bail!(
                "{} is not a Dota 2 install (no game/dota/cfg inside); pass the `dota 2 beta` directory",
                dir.display()
            );
        }
        return Ok(vec![dir]);
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let found = find_dota_dirs(Path::new(&home));
    if found.is_empty() {
        bail!(
            "could not find Dota 2 in any Steam library under {}; pass --dota-dir",
            Path::new(&home).display()
        );
    }
    Ok(found)
}

pub fn cfg_dir(dota_dir: &Path) -> PathBuf {
    dota_dir.join("game/dota/cfg")
}

pub fn console_log(dota_dir: &Path) -> PathBuf {
    dota_dir.join("game/dota/console.log")
}

fn gsi_path(dota_dir: &Path) -> PathBuf {
    cfg_dir(dota_dir)
        .join("gamestate_integration")
        .join(gsi::CFG_NAME)
}

pub fn autoexec_block(cast_key: &str, play_key: &str) -> String {
    format!(
        "{BEGIN} (managed by `servujatkot dota install`; edits inside are overwritten)\n\
         bind \"{cast_key}\" \"exec servujatkot_cast\"\n\
         bind \"{play_key}\" \"exec servujatkot_play\"\n\
         {END}\n"
    )
}

/// Byte range of our block, including the END line's newline.
fn block_range(text: &str) -> Option<(usize, usize)> {
    let start = text.find(BEGIN)?;
    let end_marker = start + text[start..].find(END)?;
    let end = text[end_marker..]
        .find('\n')
        .map_or(text.len(), |n| end_marker + n + 1);
    Some((start, end))
}

/// `text` with our block replaced by `block`, or `block` appended.
pub fn upsert_block(text: &str, block: &str) -> String {
    match block_range(text) {
        Some((s, e)) => format!("{}{block}{}", &text[..s], &text[e..]),
        None if text.is_empty() => block.to_string(),
        None => {
            let sep = if text.ends_with('\n') { "\n" } else { "\n\n" };
            format!("{text}{sep}{block}")
        }
    }
}

/// `text` without our block (and the blank line `upsert_block` put before it).
pub fn remove_block(text: &str) -> Option<String> {
    let (s, e) = block_range(text)?;
    let before = text[..s]
        .strip_suffix("\n\n")
        .map_or(&text[..s], |b| &text[..b.len() + 1]);
    Some(format!("{before}{}", &text[e..]))
}

static BIND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^\s*bind\s+"?([^"\s]+)"?\s+"?([^"]*)"?"#).unwrap());

/// Binds outside our block that use one of `keys`, as `(key, command)`.
pub fn conflicting_binds(text: &str, keys: &[&str]) -> Vec<(String, String)> {
    let outside = match block_range(text) {
        Some((s, e)) => format!("{}{}", &text[..s], &text[e..]),
        None => text.to_string(),
    };
    outside
        .lines()
        .filter_map(|l| BIND.captures(l))
        .filter(|c| keys.iter().any(|k| k.eq_ignore_ascii_case(&c[1])))
        .map(|c| (c[1].to_string(), c[2].trim().to_string()))
        .collect()
}

pub struct Write {
    pub path: PathBuf,
    pub body: String,
    pub why: &'static str,
}

#[derive(Default)]
pub struct Plan {
    /// Files read to build the plan, shown so the user can check them.
    pub reads: Vec<PathBuf>,
    pub writes: Vec<Write>,
    /// Paths to delete (uninstall).
    pub removes: Vec<PathBuf>,
    /// Directories to delete if empty once `removes` are gone.
    pub remove_empty_dirs: Vec<PathBuf>,
    pub warnings: Vec<String>,
    /// Shown after a successful apply.
    pub after: Vec<String>,
}

/// The real file behind `cfg/autoexec.cfg`. People often symlink it to a
/// dotfiles repo; we edit the target in place so the link survives.
fn autoexec_target(dota_dir: &Path) -> PathBuf {
    let link = cfg_dir(dota_dir).join("autoexec.cfg");
    link.canonicalize().unwrap_or(link)
}

/// True if a `dota2` process is running. Dota saves its settings on exit,
/// so reading or comparing them while it runs gives stale answers.
pub fn dota_running() -> bool {
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return false;
    };
    procs
        .flatten()
        .any(|p| std::fs::read_to_string(p.path().join("comm")).is_ok_and(|c| c.trim() == "dota2"))
}

fn load_snapshot(dota_dir: &Path) -> Result<Option<Snapshot>> {
    let text = read_if_exists(&cfg_dir(dota_dir).join(SNAPSHOT))?;
    if text.is_empty() {
        return Ok(None);
    }
    Snapshot::parse(&text).map(Some)
}

pub struct InstallOpts<'a> {
    pub cast_key: &'a str,
    pub play_key: &'a str,
    pub gsi_uri: &'a str,
    pub token: Option<&'a str>,
}

/// `saved_dir` is Dota's userdata cfg directory (see [`saved`]); `None` if
/// none was found, in which case only the TOML fallbacks can be restored.
pub fn plan_install(
    dota_dir: &Path,
    saved_dir: Option<&Path>,
    config: &CastingConfig,
    o: &InstallOpts,
) -> Result<Plan> {
    let dir = cfg_dir(dota_dir);
    let mut plan = Plan::default();

    let existing = load_snapshot(dota_dir)?;
    let first_install = existing.is_none();
    let mut snap = existing.unwrap_or_default();
    let saved = match saved_dir {
        Some(d) => {
            plan.reads.push(d.to_path_buf());
            Saved::load(d)?
        }
        None => {
            plan.warnings.push(
                "found no Dota settings under Steam userdata; F9 can only restore casting.toml's `play` values."
                    .into(),
            );
            Saved::default()
        }
    };
    if first_install {
        snap.source = saved_dir.map(Path::to_path_buf);
    }
    for k in [o.cast_key, o.play_key] {
        if !snap.switch_keys.iter().any(|s| s.eq_ignore_ascii_case(k)) {
            snap.switch_keys.push(k.to_uppercase());
        }
    }
    let unrestorable = snap.extend(config, &[o.cast_key, o.play_key], &saved);
    if !unrestorable.is_empty() {
        plan.warnings.push(format!(
            "no saved value or `play` fallback for {}; these stay at their casting values after F9.",
            unrestorable.join(", ")
        ));
    }
    if !first_install {
        plan.warnings.push(
            "keeping the existing snapshot of your settings; only settings new to casting.toml were recorded now."
                .into(),
        );
    }
    plan.writes.push(Write {
        path: dir.join(SNAPSHOT),
        body: snap.to_json(),
        why: "your current settings, restored by F9 and checked by uninstall",
    });
    plan.writes.push(Write {
        path: dir.join(cfg::CAST_FILE),
        body: config.render_cast(),
        why: "casting config",
    });
    plan.writes.push(Write {
        path: dir.join(cfg::PLAY_FILE),
        body: config.render_play(&snap),
        why: "playing config",
    });
    plan.writes.push(Write {
        path: gsi_path(dota_dir),
        body: gsi::install_cfg(o.gsi_uri, o.token),
        why: "game state integration",
    });

    let autoexec = autoexec_target(dota_dir);
    let link = cfg_dir(dota_dir).join("autoexec.cfg");
    if autoexec != link {
        plan.warnings.push(format!(
            "{} is a symlink; the file it points to, {}, is the one that will be edited.",
            link.display(),
            autoexec.display()
        ));
    }
    let current = read_if_exists(&autoexec)?;
    for (key, cmd) in conflicting_binds(&current, &[o.cast_key, o.play_key]) {
        plan.warnings.push(format!(
            "autoexec.cfg also binds {key} to `{cmd}`; ours comes later and wins. \
             Remove that line or pick another key with --cast-key/--play-key."
        ));
    }
    let updated = upsert_block(&current, &autoexec_block(o.cast_key, o.play_key));
    if updated != current {
        let backup = dir.join(BACKUP);
        if !current.is_empty() && block_range(&current).is_none() && !backup.exists() {
            plan.writes.push(Write {
                path: backup,
                body: current.clone(),
                why: "backup of autoexec.cfg before our first change",
            });
        }
        plan.writes.push(Write {
            path: autoexec,
            body: updated,
            why: "autoexec.cfg: key binds block",
        });
    }
    if config.unchecked() > 0 {
        plan.warnings.push(format!(
            "{} of {} settings have not been checked in the live client; see `dota lint-log`.",
            config.unchecked(),
            config.settings.len()
        ));
    }
    Ok(plan)
}

/// Removes our files and autoexec block. Refuses (unless `force`) while
/// Dota's saved settings still differ from the snapshot, because once our
/// files are gone nothing can put them back.
pub fn plan_uninstall(dota_dir: &Path, saved_dir: Option<&Path>, force: bool) -> Result<Plan> {
    let dir = cfg_dir(dota_dir);
    let mut plan = Plan::default();
    let snap = load_snapshot(dota_dir)?;

    if let (Some(snap), Some(saved_dir)) = (&snap, saved_dir) {
        plan.reads.push(saved_dir.to_path_buf());
        let switch: Vec<&str> = snap.switch_keys.iter().map(String::as_str).collect();
        let diffs = snap.differences(&Saved::load(saved_dir)?, &switch);
        if !diffs.is_empty() {
            let list = diffs.join("\n  ");
            if !force {
                bail!(
                    "Dota still has casting settings saved:\n  {list}\n\n\
                     Start Dota, press {} (playing), quit Dota, then uninstall again. \
                     --force uninstalls anyway and leaves these as they are.",
                    snap.switch_keys
                        .get(1)
                        .map_or("the play key", String::as_str)
                );
            }
            plan.warnings
                .push(format!("--force: these stay as they are:\n  {list}"));
        }
        // F9 can't restore the switch keys themselves; Dota keeps them saved.
        let cmds: Vec<String> = snap
            .switch_keys
            .iter()
            .map(|k| match snap.binds.get(k).cloned().flatten() {
                Some(cmd) => format!("bind \"{k}\" \"{cmd}\""),
                None => format!("unbind \"{k}\""),
            })
            .collect();
        plan.after.push(format!(
            "Dota keeps the switch keys bound until you change them. To put them back, run in the Dota console:\n  {}",
            cmds.join("; ")
        ));
    } else if snap.is_none() {
        plan.warnings
            .push("no snapshot found, so there is nothing to check your settings against.".into());
    }

    for p in [
        dir.join(cfg::CAST_FILE),
        dir.join(cfg::PLAY_FILE),
        dir.join(SNAPSHOT),
        dir.join(BACKUP),
        gsi_path(dota_dir),
    ] {
        if p.exists() {
            plan.removes.push(p);
        }
    }
    if let Some(gsi_dir) = gsi_path(dota_dir).parent() {
        plan.remove_empty_dirs.push(gsi_dir.to_path_buf());
    }
    let autoexec = autoexec_target(dota_dir);
    if let Some(updated) = remove_block(&read_if_exists(&autoexec)?) {
        plan.writes.push(Write {
            path: autoexec,
            body: updated,
            why: "autoexec.cfg: remove key binds block",
        });
    }
    Ok(plan)
}

fn read_if_exists(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty() && self.removes.is_empty()
    }

    pub fn describe(&self) -> String {
        let mut out = String::new();
        for r in &self.reads {
            out += &format!("read    {}  (Dota's saved settings)\n", r.display());
        }
        for w in &self.writes {
            out += &format!("write   {}  ({})\n", w.path.display(), w.why);
        }
        for p in &self.removes {
            out += &format!("remove  {}\n", p.display());
        }
        for w in &self.warnings {
            out += &format!("warning: {w}\n");
        }
        out
    }

    pub fn apply(&self) -> Result<()> {
        for w in &self.writes {
            if let Some(parent) = w.path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            // In place, not write-and-rename: the target may be a symlink's.
            std::fs::write(&w.path, &w.body)
                .with_context(|| format!("writing {}", w.path.display()))?;
        }
        for p in &self.removes {
            std::fs::remove_file(p).with_context(|| format!("removing {}", p.display()))?;
        }
        for d in &self.remove_empty_dirs {
            // Fails if not empty or already gone; both are fine.
            let _ = std::fs::remove_dir(d);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_dota(lib: &Path) -> PathBuf {
        let dir = lib.join("steamapps/common/dota 2 beta");
        std::fs::create_dir_all(dir.join("game/dota/cfg")).unwrap();
        dir
    }

    fn opts() -> InstallOpts<'static> {
        InstallOpts {
            cast_key: "F11",
            play_key: "F9",
            gsi_uri: "http://127.0.0.1:3000/gsi/dota",
            token: None,
        }
    }

    #[test]
    fn finds_dota_in_secondary_library() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".local/share/Steam");
        let other = home.path().join("games/SteamLibrary");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                root.display(),
                other.display()
            ),
        )
        .unwrap();
        let dota = fake_dota(&other);
        assert_eq!(
            find_dota_dirs(home.path()),
            vec![dota.canonicalize().unwrap()]
        );
    }

    #[test]
    fn steam_symlink_is_not_a_second_install() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".local/share/Steam");
        fake_dota(&root);
        std::fs::create_dir_all(home.path().join(".steam")).unwrap();
        std::os::unix::fs::symlink(&root, home.path().join(".steam/steam")).unwrap();
        assert_eq!(find_dota_dirs(home.path()).len(), 1);
    }

    #[test]
    fn block_upsert_is_idempotent_and_removable() {
        let user = "con_enable 1\nexec playing.cfg\n";
        let block = autoexec_block("F11", "F9");
        let once = upsert_block(user, &block);
        assert!(once.starts_with(user));
        assert_eq!(upsert_block(&once, &block), once);
        let rekeyed = upsert_block(&once, &autoexec_block("F8", "F9"));
        assert!(rekeyed.contains("bind \"F8\"") && !rekeyed.contains("bind \"F11\""));
        assert_eq!(remove_block(&once).unwrap(), user);
        assert!(remove_block(user).is_none());
    }

    #[test]
    fn conflicts_ignore_our_own_block() {
        let text = format!(
            "bind \"F11\" \"exec pimpcasting.cfg\"\nbind F12 \"exec playing.cfg\"\n\n{}",
            autoexec_block("F11", "F9")
        );
        assert_eq!(
            conflicting_binds(&text, &["f11", "F9"]),
            vec![("F11".to_string(), "exec pimpcasting.cfg".to_string())]
        );
    }

    #[test]
    fn install_edits_symlinked_autoexec_in_place_and_uninstalls() {
        let tmp = tempfile::tempdir().unwrap();
        let dota = fake_dota(tmp.path());
        let dotfiles = tmp.path().join("dotfiles/autoexec.cfg");
        std::fs::create_dir_all(dotfiles.parent().unwrap()).unwrap();
        let original = "con_enable 1\nbind \"F11\" \"exec pimpcasting.cfg\"\n";
        std::fs::write(&dotfiles, original).unwrap();
        let link = cfg_dir(&dota).join("autoexec.cfg");
        std::os::unix::fs::symlink(&dotfiles, &link).unwrap();

        let config = CastingConfig::parse(cfg::DEFAULT_TOML).unwrap();
        let plan = plan_install(&dota, None, &config, &opts()).unwrap();
        assert!(plan.warnings.iter().any(|w| w.contains("F11")));
        assert!(plan.warnings.iter().any(|w| w.contains("is a symlink")));
        plan.apply().unwrap();

        assert!(
            link.symlink_metadata().unwrap().file_type().is_symlink(),
            "link kept"
        );
        let edited = std::fs::read_to_string(&dotfiles).unwrap();
        assert!(edited.contains("exec servujatkot_cast"));
        assert_eq!(
            std::fs::read_to_string(cfg_dir(&dota).join(BACKUP)).unwrap(),
            original
        );
        assert!(gsi_path(&dota).exists());
        assert!(cfg_dir(&dota).join(cfg::CAST_FILE).exists());

        // Reinstalling changes nothing in autoexec and makes no new backup.
        let again = plan_install(&dota, None, &config, &opts()).unwrap();
        assert!(
            !again
                .writes
                .iter()
                .any(|w| w.path == dotfiles.canonicalize().unwrap())
        );

        plan_uninstall(&dota, None, false).unwrap().apply().unwrap();
        assert_eq!(std::fs::read_to_string(&dotfiles).unwrap(), original);
        assert!(!gsi_path(&dota).exists());
        assert!(!cfg_dir(&dota).join(cfg::CAST_FILE).exists());
        assert!(!cfg_dir(&dota).join(SNAPSHOT).exists());
        assert!(!cfg_dir(&dota).join(BACKUP).exists());
        assert!(!cfg_dir(&dota).join("gamestate_integration").exists());
    }

    /// Install snapshots the real values; F9 restores them; uninstall refuses
    /// until Dota has saved them back.
    #[test]
    fn snapshot_drives_play_cfg_and_guards_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        let dota = fake_dota(tmp.path());
        let saved = tmp.path().join("userdata/1/570/local/cfg");
        std::fs::create_dir_all(&saved).unwrap();
        let convars = |speed: &str| {
            format!(
                "\"config\"\n{{\n\t\"convars\"\n\t{{\n\t\t\"fps_max\"\t\t\"300\"\n\t\t\"dota_camera_speed\"\t\t\"{speed}\"\n\t}}\n}}\n"
            )
        };
        std::fs::write(saved.join("user_convars_0_slot0.vcfg"), convars("6000")).unwrap();
        std::fs::write(
            saved.join("user_keys_0_slot0.vcfg"),
            "\"config\"\n{\n\t\"bindings\"\n\t{\n\t\t\"SPACE\"\t\t\"+camhold\"\n\t}\n}\n",
        )
        .unwrap();

        let config = CastingConfig::parse(cfg::DEFAULT_TOML).unwrap();
        plan_install(&dota, Some(&saved), &config, &opts())
            .unwrap()
            .apply()
            .unwrap();
        let play = std::fs::read_to_string(cfg_dir(&dota).join(cfg::PLAY_FILE)).unwrap();
        assert!(
            play.contains("fps_max \"300\""),
            "user's value, not the TOML's 240"
        );
        assert!(play.contains("bind \"SPACE\" \"+camhold\""));
        assert!(play.contains("unbind \"MOUSE4\""), "was unbound before");

        // Dota saved the casting camera speed: uninstall must refuse.
        std::fs::write(saved.join("user_convars_0_slot0.vcfg"), convars("3000")).unwrap();
        let err = plan_uninstall(&dota, Some(&saved), false)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("dota_camera_speed is 3000, was 6000"), "{err}");
        assert!(plan_uninstall(&dota, Some(&saved), true).is_ok());

        // Back to playing values: uninstall goes through and explains F11/F9.
        std::fs::write(saved.join("user_convars_0_slot0.vcfg"), convars("6000")).unwrap();
        let plan = plan_uninstall(&dota, Some(&saved), false).unwrap();
        assert!(plan.after[0].contains("unbind \"F11\""));
    }
}
