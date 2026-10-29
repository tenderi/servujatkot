# servujatkot

Casting tools (Dota 2 first, CS later). One Rust binary: CLI + axum server for GSI and OBS overlays. Read `docs/DESIGN.md` first.

- No Node, no frontend build: maud server-rendering, SSE fragment pushes, a tiny vanilla `static/overlay.js`. CSS/JS/data are `include_str!`'d.
- The casting config lives in `configs/dota/casting.toml`, not in .cfg files. Set `checked` only from a real `dota lint-log` run.
- `data/dota/sample_draft.json` is a trimmed real capture (draft + league), plus a test `auth` token.
- The original pimpcasting repo has no license; don't copy text from it.
- Dota 2 **is** installed on the dev workstation (`~/.local/share/Steam/steamapps/common/dota 2 beta`), Its old pimpcasting symlinks were removed on 2026-10-29 at the user's request; `autoexec.cfg` is now a plain file holding the user's own binds (F7 console, `+camhold`). `~/bazecor/pimpcasting` is the user's own git repo, so leave it alone. Never write into the Dota folder without the user's go-ahead: use `dota install --dry-run --dota-dir …` to check. Only the user can run the game.
- The installer must never assume a folder: discovered Dota dirs are always confirmed interactively. `--yes` requires an explicit `--dota-dir`.
- Never write Dota's own saved settings (`userdata/<account>/570/local/cfg/*.vcfg`; Steam Cloud-synced): read only. See DESIGN "Restoring the player's settings".
- Before committing: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` (CI runs the same).
