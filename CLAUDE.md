# servujatkot

Casting tools (Dota 2 first, CS later). One Rust binary: CLI + axum server for GSI and OBS overlays. Read `docs/DESIGN.md` first.

- No Node, no frontend build: maud server-rendering, SSE fragment pushes, a tiny vanilla `static/overlay.js`. CSS/JS/data are `include_str!`'d.
- The casting config lives in `configs/dota/casting.toml`, not in .cfg files. Set `checked` only from a real `dota lint-log` run.
- `data/dota/sample_draft.json` is hand-written, not captured. Don't treat its field names as confirmed.
- The original pimpcasting repo has no license; don't copy text from it.
- No Dota client on the dev workstation (Linux, `tuuletin`). In-game behaviour has to be verified by the user.
- Before committing: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` (CI runs the same).
