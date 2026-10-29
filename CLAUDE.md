# servujatkot

Casting tools (Dota 2 first, CS later). One Rust binary: CLI + axum server for GSI and OBS overlays. Read `docs/DESIGN.md` first.

- No Node, no frontend build: maud server-rendering, SSE fragment pushes, a tiny vanilla `static/overlay.js`. CSS/JS/data are `include_str!`'d.
- The casting config lives in `configs/dota/casting.toml`, not in .cfg files. Set `checked` only from a real `dota lint-log` run.
- `data/dota/sample_draft.json` is a trimmed real capture (draft, league, player names/account ids), plus a test `auth` token. `sample_player_heroes.json` is matching OpenDota stats, so `/demo` and tests stay offline.
- Live testing loop: the user spectates (replays work, and send the full draft) with `servujatkot serve --record <file>.jsonl`; Claude reads the recording and fixes from it. The user's recordings are in `target/release/` (e.g. `servujatkot-session.jsonl`, PARIVISION vs Team Yandex, ends mid-draft). The user runs `target/release/servujatkot`, so rebuild it (`cargo build --release`) after changes and tell them to restart `serve`.
- Check overlay changes visually: headless Chromium screenshots of the `/demo` pages at 1920×1080 (`--virtual-time-budget=3000 --timeout=4000`; a live SSE page never finishes loading without the timeout).
- The user's casts are **amateur games, not pro**. Don't rely on pro-only data (team rosters, pro match APIs). Player guesses use each account's all-time OpenDota history (`serve --player-days` narrows it). Private profiles return nothing. OpenDota rejects requests with no User-Agent.
- The original pimpcasting repo has no license; don't copy text from it.
- Dota 2 **is** installed on the dev workstation (`~/.local/share/Steam/steamapps/common/dota 2 beta`), Its old pimpcasting symlinks were removed on 2026-10-29 at the user's request; `autoexec.cfg` is now a plain file holding the user's own binds (F7 console, `+camhold`). `~/bazecor/pimpcasting` is the user's own git repo, so leave it alone. Never write into the Dota folder without the user's go-ahead: use `dota install --dry-run --dota-dir …` to check. Only the user can run the game.
- The installer must never assume a folder: discovered Dota dirs are always confirmed interactively. `--yes` requires an explicit `--dota-dir`.
- Never write Dota's own saved settings (`userdata/<account>/570/local/cfg/*.vcfg`; Steam Cloud-synced): read only. See DESIGN "Restoring the player's settings".
- Releases: push a `v*` tag; `release.yml` builds Linux musl + Windows and publishes a GitHub release.
- Before committing: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` (CI runs the same).
