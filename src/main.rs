mod dota;
mod record;
mod server;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use dota::cfg::{self, CastingConfig};
use dota::install;

#[derive(Parser)]
#[command(
    version,
    about = "Casting tools: observer configs, game state feeds, OBS overlays"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the GSI receiver and overlay server.
    Serve {
        #[arg(long, default_value = "127.0.0.1:3000", env = "SERVUJATKOT_BIND")]
        bind: SocketAddr,
        /// Require this token in GSI posts (must match `dota install --token`).
        #[arg(long, env = "SERVUJATKOT_TOKEN")]
        token: Option<String>,
        /// Append received game state to this file (JSON lines) for later analysis.
        #[arg(long)]
        record: Option<PathBuf>,
        /// Don't look up players' hero history on OpenDota.
        #[arg(long)]
        no_player_stats: bool,
        /// Count only players' games from the last N days (0 = all time).
        #[arg(long, default_value_t = 365)]
        player_days: u32,
    },
    /// Dota 2 tools.
    Dota {
        #[command(subcommand)]
        cmd: DotaCmd,
    },
}

#[derive(Subcommand)]
enum DotaCmd {
    /// Install the configs, GSI cfg and autoexec binds into Dota 2.
    Install {
        /// The `dota 2 beta` directory. Found in your Steam libraries if omitted.
        #[arg(long, env = "SERVUJATKOT_DOTA_DIR")]
        dota_dir: Option<PathBuf>,
        /// Source TOML. Defaults to the one built into this binary.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value = "F11")]
        cast_key: String,
        #[arg(long, default_value = "F9")]
        play_key: String,
        #[arg(long, default_value = "http://127.0.0.1:3000/gsi/dota")]
        uri: String,
        /// Token Dota sends with game state (must match `serve --token`).
        #[arg(long, env = "SERVUJATKOT_TOKEN")]
        token: Option<String>,
        /// Show what would be written without writing anything.
        #[arg(long)]
        dry_run: bool,
        /// Don't ask for confirmation (requires --dota-dir).
        #[arg(long, short)]
        yes: bool,
    },
    /// Remove everything `install` added.
    Uninstall {
        #[arg(long, env = "SERVUJATKOT_DOTA_DIR")]
        dota_dir: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        /// Don't ask for confirmation (requires --dota-dir).
        #[arg(long, short)]
        yes: bool,
        /// Uninstall even though Dota still has casting settings saved.
        #[arg(long)]
        force: bool,
    },
    /// Print the generated casting config (or the playing one) without installing.
    Cfg {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        play: bool,
    },
    /// Check a `-condebug` console.log for settings the client rejected.
    LintLog {
        /// Defaults to console.log in the Dota install.
        log: Option<PathBuf>,
        #[arg(long, env = "SERVUJATKOT_DOTA_DIR")]
        dota_dir: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    match Cli::parse().cmd {
        Cmd::Serve {
            bind,
            token,
            record,
            no_player_stats,
            player_days,
        } => {
            let recorder = record.as_deref().map(record::Recorder::open).transpose()?;
            if let Some(p) = &record {
                tracing::info!("recording game state to {}", p.display());
            }
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .with_context(|| format!("binding {bind}"))?;
            tracing::info!("listening on http://{bind} (control page at /control)");
            if token.is_none() {
                tracing::warn!("no --token: accepting GSI posts from anyone who can reach {bind}");
            }
            let opendota = (!no_player_stats)
                .then(|| dota::players::OpenDota::new(player_days))
                .transpose()?;
            let app = server::App::new(token, recorder, opendota);
            axum::serve(listener, server::router(app)).await?;
        }
        Cmd::Dota { cmd } => dota_cmd(cmd)?,
    }
    Ok(())
}

fn load_config(path: Option<&PathBuf>) -> Result<CastingConfig> {
    match path {
        Some(p) => CastingConfig::parse(
            &std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        ),
        None => CastingConfig::parse(cfg::DEFAULT_TOML),
    }
}

fn dota_cmd(cmd: DotaCmd) -> Result<()> {
    match cmd {
        DotaCmd::Install {
            dota_dir,
            config,
            cast_key,
            play_key,
            uri,
            token,
            dry_run,
            yes,
        } => {
            ensure_dota_closed()?;
            let dir = pick_dota_dir(dota_dir, yes)?;
            let c = load_config(config.as_ref())?;
            let opts = install::InstallOpts {
                cast_key: &cast_key,
                play_key: &play_key,
                gsi_uri: &uri,
                token: token.as_deref(),
            };
            let saved_dir = pick_saved_dir();
            let plan = install::plan_install(&dir, saved_dir.as_deref(), &c, &opts)?;
            if run_plan(&plan, dry_run, yes)? {
                println!(
                    "\nInstalled into {}.\nFor the overlay, add -gamestateintegration to Dota's Steam launch options.\nStart Dota: {cast_key} loads the casting setup, {play_key} puts your settings back.",
                    dir.display()
                );
            }
        }
        DotaCmd::Uninstall {
            dota_dir,
            dry_run,
            yes,
            force,
        } => {
            ensure_dota_closed()?;
            let dir = pick_dota_dir(dota_dir, yes)?;
            let saved_dir = pick_saved_dir();
            let plan = install::plan_uninstall(&dir, saved_dir.as_deref(), force)?;
            if plan.is_empty() {
                println!("Nothing of ours in {}.", dir.display());
                return Ok(());
            }
            if run_plan(&plan, dry_run, yes)? {
                println!("\nRemoved. Restart Dota 2 to drop the binds.");
            }
        }
        DotaCmd::Cfg { config, play } => {
            let c = load_config(config.as_ref())?;
            print!(
                "{}",
                if play {
                    c.render_play(&dota::saved::Snapshot::default())
                } else {
                    c.render_cast()
                }
            );
        }
        DotaCmd::LintLog {
            log,
            dota_dir,
            config,
        } => {
            let log = match log {
                Some(l) => l,
                None => install::console_log(&pick_dota_dir(dota_dir, false)?),
            };
            let c = load_config(config.as_ref())?;
            let text = std::fs::read(&log).with_context(|| {
                format!(
                    "reading {} (start Dota with -condebug to get one)",
                    log.display()
                )
            })?;
            let report = cfg::lint_log(&String::from_utf8_lossy(&text), &c.keys());
            print_lint(&report, &c.keys());
        }
    }
    Ok(())
}

/// Shows the plan and applies it after confirmation. Returns whether it was applied.
fn run_plan(plan: &install::Plan, dry_run: bool, yes: bool) -> Result<bool> {
    print!("{}", plan.describe());
    if dry_run {
        println!("\n(dry run: nothing written)");
        return Ok(false);
    }
    if !yes && !confirm("\nWrite these changes?")? {
        println!("Nothing written.");
        return Ok(false);
    }
    plan.apply()?;
    for a in &plan.after {
        println!("\n{a}");
    }
    Ok(true)
}

/// Dota's saved-settings directory, newest first. Shown in the plan the user
/// confirms, so no separate prompt.
fn pick_saved_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let roots = install::steam_roots(std::path::Path::new(&home));
    dota::saved::find_saved_dirs(&roots).into_iter().next()
}

/// The Dota folder to use. An explicit --dota-dir is taken as is; a
/// discovered one is always shown to the user to confirm or choose from.
fn pick_dota_dir(given: Option<PathBuf>, yes: bool) -> Result<PathBuf> {
    let explicit = given.is_some();
    let mut found = install::dota_dir_candidates(given)?;
    if explicit {
        return Ok(found.remove(0));
    }
    if yes {
        anyhow::bail!("--yes needs --dota-dir: a discovered folder has to be confirmed");
    }
    if found.len() == 1 {
        println!("Found Dota 2 at:\n  {}", found[0].display());
        if confirm("Is this the right folder?")? {
            return Ok(found.remove(0));
        }
        anyhow::bail!("not confirmed; pass the right folder with --dota-dir");
    }
    println!("Found several Dota 2 installs:");
    for (i, d) in found.iter().enumerate() {
        println!("  {}) {}", i + 1, d.display());
    }
    let answer = ask(&format!("Which one? [1-{}, empty to cancel]", found.len()))?;
    match answer.parse::<usize>() {
        Ok(n) if (1..=found.len()).contains(&n) => Ok(found.swap_remove(n - 1)),
        _ => anyhow::bail!("no folder chosen; pass one with --dota-dir"),
    }
}

fn ensure_dota_closed() -> Result<()> {
    if install::dota_running() {
        anyhow::bail!(
            "Dota 2 is running. Quit it first: it saves its settings on exit, and those are what this reads."
        );
    }
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    let a = ask(&format!("{question} [y/N]"))?;
    Ok(matches!(a.to_lowercase().as_str(), "y" | "yes"))
}

fn ask(prompt: &str) -> Result<String> {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("need an answer but stdin is not a terminal; pass --dota-dir (and --yes)");
    }
    print!("{prompt} ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn print_lint(r: &cfg::LintReport, keys: &BTreeSet<&str>) {
    if r.unknown.is_empty() && r.cheat.is_empty() {
        println!(
            "No rejected commands found. If the log doesn't contain \"servujatkot: cast config loaded\", the cfg never ran."
        );
        return;
    }
    for k in &r.unknown {
        let ours = if keys.contains(k.as_str()) {
            "  <- in casting.toml"
        } else {
            ""
        };
        println!("unknown command: {k}{ours}");
    }
    for k in &r.cheat {
        println!("cheat-protected: {k}  <- in casting.toml");
    }
}
