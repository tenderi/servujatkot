# servujatkot

Casting tools for community esports broadcasts, in one binary with no install
step. It currently does Dota 2:

- **Casting config**: an up-to-date take on the old *pimpcasting* observer config.
  Settings live in [`configs/dota/casting.toml`](configs/dota/casting.toml) and
  are rendered into a cast cfg and a play cfg, with binds to switch between
  them. A checker reads the game's console log and flags any setting the
  current client no longer accepts.
- **Draft overlay**: a picks/bans/timer bar for OBS, fed live by Dota 2's
  Game State Integration (GSI) while you spectate.

Counter-Strike and other games are planned; see [docs/DESIGN.md](docs/DESIGN.md).

## Casting config

```sh
# Writes servujatkot_cast.cfg and servujatkot_play.cfg, and prints the autoexec lines
servujatkot dota cfg --out "C:\Program Files (x86)\Steam\steamapps\common\dota 2 beta\game\dota\cfg"
```

Paste the printed lines into `autoexec.cfg` in the same folder. Then **F11**
loads the casting setup and **F9** goes back to playing (change these with
`--cast-key` and `--play-key`). F12 is avoided because Steam uses it for
screenshots. If the autoexec doesn't run, add `+exec autoexec` to Dota's launch
options.

Review the `play` values in the TOML first: they're what you get back when you
switch to playing, so they should match how you play. To edit the config, copy
the TOML, change it, and pass `--config yours.toml`.

**Checking it against the current client.** Add `-condebug` to the launch
options, start Dota, press F11, and quit. Then:

```sh
servujatkot dota lint-log ".../dota 2 beta/game/dota/console.log"
```

This lists every command the client reported as unknown or cheat-protected.
Remove or fix those in the TOML, and set `checked = "<date>"` on the settings
that worked.

**Camera hotkeys** (`dota_camera_set_lookatpos`) for runes, Roshan and
similar spots are in the TOML but have no coordinates yet. The map has moved
too much since the old configs. Give each one a `pos` and a `key` to turn it on.

## Draft overlay

```sh
# 1. Tell Dota where to send game state (restart Dota afterwards)
servujatkot dota gsi-install --dota-dir ".../dota 2 beta" --token SOME-SECRET

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

Windows and Linux binaries: run the *Build release binaries* workflow
(`gh workflow run release.yml`) and download the artifacts.

Hero data comes from [odota/dotaconstants](https://github.com/odota/dotaconstants) (MIT); see `data/dota/NOTICE`.
