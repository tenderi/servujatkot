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

const BEGIN: &str = "// >>> servujatkot";
const END: &str = "// <<< servujatkot";
const BACKUP: &str = "autoexec.cfg.servujatkot-backup";

/// Steam roots to look in, most common first: native, the `~/.steam`
/// symlinks, Flatpak and Snap.
fn steam_roots(home: &Path) -> Vec<PathBuf> {
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
    pub writes: Vec<Write>,
    /// Paths to delete (uninstall).
    pub removes: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

/// The real file behind `cfg/autoexec.cfg`. People often symlink it to a
/// dotfiles repo; we edit the target in place so the link survives.
fn autoexec_target(dota_dir: &Path) -> PathBuf {
    let link = cfg_dir(dota_dir).join("autoexec.cfg");
    link.canonicalize().unwrap_or(link)
}

pub struct InstallOpts<'a> {
    pub cast_key: &'a str,
    pub play_key: &'a str,
    pub gsi_uri: &'a str,
    pub token: Option<&'a str>,
}

pub fn plan_install(dota_dir: &Path, config: &CastingConfig, o: &InstallOpts) -> Result<Plan> {
    let dir = cfg_dir(dota_dir);
    let mut plan = Plan::default();
    plan.writes.push(Write {
        path: dir.join(cfg::CAST_FILE),
        body: config.render_cast(),
        why: "casting config",
    });
    plan.writes.push(Write {
        path: dir.join(cfg::PLAY_FILE),
        body: config.render_play(),
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

pub fn plan_uninstall(dota_dir: &Path) -> Result<Plan> {
    let dir = cfg_dir(dota_dir);
    let mut plan = Plan::default();
    for p in [
        dir.join(cfg::CAST_FILE),
        dir.join(cfg::PLAY_FILE),
        gsi_path(dota_dir),
    ] {
        if p.exists() {
            plan.removes.push(p);
        }
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
    pub fn describe(&self) -> String {
        let mut out = String::new();
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
        let plan = plan_install(&dota, &config, &opts()).unwrap();
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
        let again = plan_install(&dota, &config, &opts()).unwrap();
        assert!(
            !again
                .writes
                .iter()
                .any(|w| w.path == dotfiles.canonicalize().unwrap())
        );

        plan_uninstall(&dota).unwrap().apply().unwrap();
        assert_eq!(std::fs::read_to_string(&dotfiles).unwrap(), original);
        assert!(!gsi_path(&dota).exists());
        assert!(!cfg_dir(&dota).join(cfg::CAST_FILE).exists());
    }
}
