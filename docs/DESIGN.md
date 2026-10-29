# Design

## Shape

One Rust binary that runs on the casting PC. Linux is the target for now; the code avoids Linux-only assumptions outside `dota/install.rs`.

- **CLI subcommands per game** (`servujatkot dota …`, later `servujatkot cs …`)
  for one-off jobs: installing configs and the GSI cfg into the game, linting logs.
- **`serve`**: an axum HTTP server on `127.0.0.1:3000`. It accepts game-state
  POSTs from the game client and serves the overlays as OBS browser sources,
  plus a `/control` page for the operator.

There is no Node and no build step. Pages are rendered on the server with maud.
Live updates use Server-Sent Events: on each GSI packet the server re-renders
the overlay fragment and pushes it, and a five-line `static/overlay.js` swaps it
in. CSS and JS are compiled into the binary.

State is in memory only: the last GSI payload and the team names. Nothing needs
to survive a restart yet.

## Dota 2

### Casting config

The original [pimpcasting](https://github.com/Pimpmuckl/pimpcasting) (v0.7.1,
2020) is unlicensed, so nothing here is copied from it. Our config was written
fresh from the same ideas: slow camera, no edge-pan, WASD camera, assisted
camera on MOUSE4 or held SHIFT, and separate cast and play cfgs.

The config is **data, not a .cfg file**. Each setting carries both its cast and
play values, a note, and a `checked` date. That makes keeping it current a
review of a TOML diff. The `checked` dates are honest only if they come from
`dota lint-log` against a real client.

Known gaps, as of 2026-10-29:
- No setting has been checked against the current client yet.
- Camera spot coordinates (`[[spot]]`) are empty. The rune, Roshan pit,
  Tormentor and Lotus locations have all moved since 2020.
- `lint-log`'s regexes for "unknown command" and cheat-protected lines are
  guesses at Source 2's wording. Fix them against a real `console.log`.

### Installer

`dota/install.rs` searches the Steam roots (native, `~/.steam` links, Flatpak,
Snap) and every library in `libraryfolders.vdf`, deduplicating by canonical
path. It **never picks a folder silently**: a discovered folder is shown for
confirmation, and if there are several the user chooses. Work is computed as a
`Plan` first, so `--dry-run` prints it and the user approves it before
anything is written. The autoexec change is a marker-delimited block, so it's
idempotent and uninstall removes exactly that. Writes happen in place rather
than by temp-file-and-rename, so a symlinked `autoexec.cfg` stays a symlink.

### Restoring the player's settings

Dota writes every non-default convar and console bind to
`<Steam>/userdata/<account>/570/local/cfg/` (`user_convars_0_slot0.vcfg`,
`machine_convars.vcfg`, `user_keys_0_slot0.vcfg`, all KeyValues) on exit, so
whatever the cast cfg sets persists. Restoring files alone is therefore not
enough. `dota/saved.rs` parses those files. On the first install it records
the current value of every convar and key we touch into
`servujatkot_snapshot.json`, and only ever adds to it, because later runs
may see casting values. The play cfg is rendered from the snapshot. Uninstall
compares the saved files against it and refuses while they differ. We never
write Dota's own files: they're Steam Cloud-synced. Both commands refuse to
run while a `dota2` process exists.

Open gap: a convar absent from the saved files is at its default, which we
can't read. It falls back to casting.toml's `play`, or stays unrestorable.
Typing a convar's name in the console prints its default, so `lint-log` could
learn the defaults from a `-condebug` log. That would close the gap.

### Game State Integration

The client reads `game/dota/cfg/gamestate_integration/gamestate_integration_*.cfg`
at startup and POSTs JSON to its `uri`. While **spectating** (DotaTV, a lobby
broadcaster slot, or a replay) it includes every player and a `draft` block.
While playing, it includes only your own hero, so overlays need a spectator
client.

Draft fields (from the Dota2GSI library; not yet seen in a real capture):
`activeteam` (2 = Radiant, 3 = Dire), `pick` (bool), `activeteam_time_remaining`,
`radiant_bonus_time`, `dire_bonus_time`, and `team2` / `team3`, each holding
`home_team` and `pickN_id` / `pickN_class`. Bans are parsed from `banN_id` /
`banN_class` **in case** the client sends them. If it doesn't, bans need a
manual input on `/control` (planned).

`data/dota/sample_draft.json` is **hand-written** from those field names. It
powers the tests and `/dota/draft/demo`. Replace it with a real capture
(`/api/dota/state` during a Captains Mode draft) as soon as there is one.

Auth: when `--token` is set, posts whose `auth.token` doesn't match get a 401
and are counted on `/control`. The server binds to loopback by default.
If you run Dota and OBS on different machines, bind to the LAN address and keep
the token.

### Hero data

`data/dota/heroes.json` is a trimmed copy of dotaconstants' `build/heroes.json`
(id, internal name, display name, primary attribute). To refresh it after a new
hero:

```sh
curl -sL https://raw.githubusercontent.com/odota/dotaconstants/master/build/heroes.json \
| python3 -c 'import json,sys; h=json.load(sys.stdin); json.dump([{"id":v["id"],"class":v["name"].removeprefix("npc_dota_hero_"),"name":v["localized_name"],"attr":v["primary_attr"]} for v in sorted(h.values(),key=lambda v:v["id"])],sys.stdout,indent=0,ensure_ascii=False)' \
> data/dota/heroes.json
```

Portraits load at runtime from `cdn.steamstatic.com/apps/dota2/images/dota_react/heroes/<class>.png`.

## Roadmap

Dota 2, roughly in order:
1. Check the casting config against a live client; calibrate camera spots.
2. Capture a real draft payload; confirm or fix bans; add manual ban input on `/control`.
3. Scoreboard / top bar (kills, net worth lead, clock) from the same feed.
4. Player cams overlay: the hero and items of the selected player.
5. Roshan/Aegis and buyback timers.

Other games:
- **Counter-Strike 2** has GSI too (`csgo/cfg/gamestate_integration_*.cfg`,
  same POST model). It would add `src/cs/` and a `/gsi/cs` route next to Dota's,
  reusing the server, overlay page shell and SSE plumbing. Spectator configs
  (`spec_*` convars, `cl_draw_only_deathnotices`, and so on) would follow the
  same TOML → cfg pattern.
