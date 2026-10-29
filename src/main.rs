mod dota;
mod server;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use dota::cfg::{self, CastingConfig};

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
        /// Require this token in GSI posts (must match `dota gsi-install --token`).
        #[arg(long, env = "SERVUJATKOT_TOKEN")]
        token: Option<String>,
    },
    /// Dota 2 tools.
    Dota {
        #[command(subcommand)]
        cmd: DotaCmd,
    },
}

#[derive(Subcommand)]
enum DotaCmd {
    /// Generate the casting and playing console configs.
    Cfg {
        /// Source TOML. Defaults to the one built into this binary.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Directory to write into, usually `<dota 2 beta>/game/dota/cfg`.
        /// Without it, the cast config is printed.
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "F11")]
        cast_key: String,
        #[arg(long, default_value = "F9")]
        play_key: String,
    },
    /// Install the Game State Integration cfg into a Dota 2 install.
    GsiInstall {
        /// The `dota 2 beta` directory. Without it, the cfg is printed.
        #[arg(long)]
        dota_dir: Option<PathBuf>,
        #[arg(long, default_value = "http://127.0.0.1:3000/gsi/dota")]
        uri: String,
        #[arg(long, env = "SERVUJATKOT_TOKEN")]
        token: Option<String>,
    },
    /// Check a `-condebug` console.log for settings the client rejected.
    LintLog {
        log: PathBuf,
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
        Cmd::Serve { bind, token } => {
            let listener = tokio::net::TcpListener::bind(bind)
                .await
                .with_context(|| format!("binding {bind}"))?;
            tracing::info!("listening on http://{bind} (control page at /control)");
            if token.is_none() {
                tracing::warn!("no --token: accepting GSI posts from anyone who can reach {bind}");
            }
            axum::serve(listener, server::router(server::App::new(token))).await?;
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
        DotaCmd::Cfg {
            config,
            out,
            cast_key,
            play_key,
        } => {
            let c = load_config(config.as_ref())?;
            let Some(dir) = out else {
                print!("{}", c.render_cast());
                return Ok(());
            };
            for (name, body) in [
                (cfg::CAST_FILE, c.render_cast()),
                (cfg::PLAY_FILE, c.render_play()),
            ] {
                let path = dir.join(name);
                std::fs::write(&path, body)
                    .with_context(|| format!("writing {}", path.display()))?;
                println!("wrote {}", path.display());
            }
            if c.unchecked() > 0 {
                println!(
                    "\n{} of {} settings have not been checked in the live client; see `dota lint-log`.",
                    c.unchecked(),
                    c.settings.len()
                );
            }
            println!("\nAdd to autoexec.cfg in the same directory:\n");
            print!("{}", cfg::autoexec_snippet(&cast_key, &play_key));
        }
        DotaCmd::GsiInstall {
            dota_dir,
            uri,
            token,
        } => match dota_dir {
            None => print!("{}", dota::gsi::install_cfg(&uri, token.as_deref())),
            Some(dir) => {
                let path = dota::gsi::install(&dir, &uri, token.as_deref())?;
                println!(
                    "wrote {}\nRestart Dota 2 for it to take effect.",
                    path.display()
                );
            }
        },
        DotaCmd::LintLog { log, config } => {
            let c = load_config(config.as_ref())?;
            let text = std::fs::read(&log).with_context(|| format!("reading {}", log.display()))?;
            let report = cfg::lint_log(&String::from_utf8_lossy(&text), &c.keys());
            print_lint(&report, &c.keys());
        }
    }
    Ok(())
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
