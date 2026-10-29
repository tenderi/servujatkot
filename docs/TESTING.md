# Live testing loop

Dota runs on the dev machine, so a test session leaves everything readable on
disk. Claude can pick it up afterwards and make fixes.

## Before

1. Steam launch options: `-gamestateintegration -condebug`.
2. Quit Dota, then run `servujatkot dota install` (see README).
3. `servujatkot serve --record ~/servujatkot-session.jsonl`, and leave it running.

## In the game

Note anything that felt wrong. Then:

1. Press **F11**. Try the camera, MOUSE4, SHIFT, SPACE and mouse-wheel zoom.
2. Spectate a **Captains Mode** game from the draft onwards. Keep
   http://127.0.0.1:3000/dota/draft open, in OBS or a browser.
3. Press **F9**, then quit Dota (it saves its settings on exit).

## After: what Claude reads

| Source | Answers |
|---|---|
| `servujatkot dota lint-log` (`console.log`) | Settings the client rejected, and errors from the cfgs |
| `~/servujatkot-session.jsonl` | What GSI really sends: draft fields, bans, team names. Replaces `data/dota/sample_draft.json` |
| `servujatkot dota uninstall --dry-run` | Whether F9 restored your settings (it lists anything still at casting values) |
| headless Chromium on `/dota/draft` | The overlay, re-rendered from the captured payloads |

Claude can't see the game or OBS. Camera feel, key behaviour and overlay
placement need your notes.
