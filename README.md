# servujatkot

Casting tools for community esports broadcasts, in one binary. It currently
does Dota 2, on Linux:

- **Casting config**: an up-to-date take on the old *pimpcasting* observer
  config, switched on with F11 and off with F9. F9 puts back your own settings.
- **Draft overlays** for OBS, fed live by Dota 2's Game State Integration (GSI)
  while you spectate: a picks/bans bar over gameplay, and a fullscreen draft
  scene with animated hero renders.

Counter-Strike and other games are planned; see [docs/DESIGN.md](docs/DESIGN.md). Testing with a live client: [docs/TESTING.md](docs/TESTING.md).

## Quick start

### 1. Build

You need a Rust toolchain (`sudo pacman -S rustup && rustup default stable`).

```sh
git clone https://github.com/tenderi/servujatkot && cd servujatkot
cargo build --release
cp target/release/servujatkot ~/.local/bin/   # or run it from target/release/
```

### 2. Install into Dota

Quit Dota first, then:

```sh
servujatkot dota install --dry-run   # optional: list what it would write
servujatkot dota install
```

It finds Dota in your Steam libraries and asks you to confirm the folder. It
then lists the files it will write and asks again.

### 3. Set Steam launch options

In Steam, right-click Dota 2, choose **Properties**, and set **Launch Options** to:

```
-gamestateintegration -condebug
```

- `-gamestateintegration` is required for the overlay; without it Dota sends nothing.
- `-condebug` writes `console.log`, which `dota lint-log` reads (step 6).
- If F11 does nothing in game, add `+exec autoexec` too.

### 4. Cast

Start Dota. **F11** loads the casting setup and **F9** puts your own settings
back. Main casting keys: MOUSE4 toggles assisted camera, holding SHIFT turns it
on while held, and holding SPACE follows the selected hero.

### 5. Run the draft overlay

```sh
servujatkot serve
```

Leave it running while you cast, then:

1. Open http://127.0.0.1:3000/control in a browser. League matches fill in
   team names, tags and the series score by themselves; type names there to
   override them.
2. In OBS, add a **Browser** source, 1920×1080, with one of these URLs:
   - `http://127.0.0.1:3000/dota/draft`: a bar along the bottom. The page is
     transparent, so put it above the game capture.
   - `http://127.0.0.1:3000/dota/draft/full`: a fullscreen draft scene with
     animated hero art. Add `?still=1` for still images if OBS stutters.
3. Spectate a Captains Mode game in Dota: a live game in Watch, or a lobby slot
   as broadcaster. The control page should show packets arriving and
   `Draft data: yes`, and the bar appears during the draft.

**Who plays each pick.** When a hero is picked, the overlays name the player
most likely to play it, guessed from each player's public games (all time) on
[OpenDota](https://www.opendota.com) (no account needed). Once the player
actually takes the hero, the real name replaces the guess. The control
page lists each player's lookup status. Players with no public data can turn on
**Expose Public Match Data** in Dota's settings; it counts for games played
after that. Use `serve --player-days 365` to count only the last year, or
`--no-player-stats` to turn lookups off.

To line up the layout without a game, add `/demo` to either path
(`/dota/draft/demo`, `/dota/draft/full/demo`); it shows a real mid-draft moment.
Hero portraits load from Valve's CDN, so OBS needs internet access.

### 6. Check the config against the current client

Do this once, and again after big Dota patches. Start Dota, press F11, quit,
then run:

```sh
servujatkot dota lint-log
```

It lists any setting the client rejected. Report those, or fix them in
[`configs/dota/casting.toml`](configs/dota/casting.toml).

## Token (`--token`)

A token is optional. Use it when Dota and OBS run on **different machines**.
It's a password you make up, given to both sides:

```sh
servujatkot dota install --token "$(openssl rand -hex 16)"   # note the value
servujatkot serve --bind 0.0.0.0:3000 --token <the same value>
```

Dota then sends the token with every update, and the server rejects updates
without it; the control page counts the rejections. With the default
`127.0.0.1` bind, only your own machine can reach the server, so you can skip
it. On the other machine, also run `dota install --uri http://<server-ip>:3000/gsi/dota`.

## Your settings, and uninstalling

Dota saves settings changed from the console under
`~/.local/share/Steam/userdata/<account>/570/local/cfg/`, so casting values
outlast the session. Before the first install, servujatkot records your
current values in `servujatkot_snapshot.json`; F9 restores exactly those. A
setting Dota has never saved falls back to the `play` value in `casting.toml`.
Settings with neither are named at install and keep their casting values.

```sh
servujatkot dota uninstall
```

This removes every file and the autoexec block it added. It refuses while Dota
still has casting values saved: start Dota, press F9, quit, then run it again.
Afterwards it prints a console line that puts your old F11/F9 binds back.

## What gets written

In `<dota 2 beta>/game/dota/cfg/`:

- `servujatkot_cast.cfg` and `servujatkot_play.cfg`
- `servujatkot_snapshot.json`
- `gamestate_integration/gamestate_integration_servujatkot.cfg`
- a marked block in `autoexec.cfg` binding the switch keys. The original is
  backed up once to `autoexec.cfg.servujatkot-backup`. Use `--cast-key` /
  `--play-key` to pick other keys.

To change the casting config, copy `configs/dota/casting.toml`, edit it, and
run `dota install --config yours.toml`.

## Releases

Pushing a `v*` tag builds Linux (and untested Windows) binaries and publishes
them as a GitHub release:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

`gh workflow run release.yml` builds the same binaries as workflow artifacts
only.

Hero data comes from [odota/dotaconstants](https://github.com/odota/dotaconstants) (MIT); see `data/dota/NOTICE`.
