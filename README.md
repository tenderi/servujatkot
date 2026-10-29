# servujatkot

Casting tools for community esports broadcasts, in one binary. It currently
does Dota 2, on Linux:

- **Casting config**: an up-to-date take on the old *pimpcasting* observer config.
  Settings live in [`configs/dota/casting.toml`](configs/dota/casting.toml) and
  are rendered into a cast cfg and a play cfg, with binds to switch between
  them. A checker reads the game's console log and flags any setting the
  current client no longer accepts.
- **Draft overlay**: a picks/bans/timer bar for OBS, fed live by Dota 2's
  Game State Integration (GSI) while you spectate.

Counter-Strike and other games are planned; see [docs/DESIGN.md](docs/DESIGN.md).

## Install into Dota 2 (Linux)

```sh
servujatkot dota install --dry-run   # show what it would write
servujatkot dota install             # do it, after confirming
```

It searches your Steam libraries for `dota 2 beta`. That covers native, Flatpak
and Snap Steam, plus every library in `libraryfolders.vdf`. It then **asks you
to confirm the folder it found**, or to pick one if there are several. Next it
lists every file it will write and asks once more. Pass `--dota-dir` to name the
folder yourself; `--yes` skips the prompts and only works together with
`--dota-dir`.

It writes:

- `game/dota/cfg/servujatkot_cast.cfg` and `servujatkot_play.cfg`
- `game/dota/cfg/gamestate_integration/gamestate_integration_servujatkot.cfg`
- a marked block in `autoexec.cfg` that binds **F11** to casting and **F9** to
  playing (`--cast-key` / `--play-key` to change them). The rest of the file is
  left alone. A copy of the original goes to `autoexec.cfg.servujatkot-backup`
  the first time. If `autoexec.cfg` is a symlink, the file it points to is
  edited in place and the link stays; the install output names that file.

It warns if your autoexec already binds the same keys. Quit Dota before running
it: Dota saves its settings when it exits, and the installer reads them.

**Getting your own settings back.** Dota saves every setting and bind changed
from the console under `~/.local/share/Steam/userdata/<account>/570/local/cfg/`,
so casting values outlive the session. Before the first install, `servujatkot`
records your current value for every setting and key the cast config touches,
in `servujatkot_snapshot.json`. That record is never overwritten. **F9 restores
exactly those values.** A setting Dota never saved (still at its default) falls
back to the `play` value in [`configs/dota/casting.toml`](configs/dota/casting.toml).
Settings with neither are named at install; they keep their casting values after F9.

`servujatkot dota uninstall` removes everything above, including the snapshot
and the autoexec backup, and removes the autoexec block exactly. It **refuses
while Dota still has casting values saved**: start Dota, press F9, quit, then
run it again (`--force` overrides). The F11/F9 binds stay saved in Dota, and the
uninstall output gives the console line that puts them back.

To change the config, copy the TOML, edit it, and pass `--config yours.toml`.
`servujatkot dota cfg [--play]` prints a generated cfg without installing it.

**Checking it against the current client.** Add `-condebug` to Dota's launch
options, start Dota, press F11, and quit. Then run `servujatkot dota lint-log`.
It reads `console.log` from the same Dota folder and lists every command the
client reported as unknown or cheat-protected. Fix or remove those in the TOML,
and set `checked = "<date>"` on the settings that worked.

**Camera hotkeys** (`dota_camera_set_lookatpos`) for runes, Roshan and similar
spots are in the TOML but have no coordinates yet. The map has moved too much
since the old configs. Give each one a `pos` and a `key` to turn it on.

## Draft overlay

```sh
# 1. Install (once) with a token; restart Dota afterwards
servujatkot dota install --token SOME-SECRET

# 2. Run the server on the casting PC
servujatkot serve --token SOME-SECRET
```

- `http://127.0.0.1:3000/control` shows feed status and lets you set team names.
- `http://127.0.0.1:3000/dota/draft` is the OBS browser source (1920×1080, transparent).
- `http://127.0.0.1:3000/dota/draft/demo` shows the same overlay filled with sample data, for lining things up before a match.

The bar slides away when there's no draft data. Hero portraits come from
Valve's CDN, so the OBS machine needs internet access.

## Building

```sh
cargo build --release
```

Linux (and, untested, Windows) binaries: run the *Build release binaries* workflow
(`gh workflow run release.yml`) and download the artifacts.

Hero data comes from [odota/dotaconstants](https://github.com/odota/dotaconstants) (MIT); see `data/dota/NOTICE`.
