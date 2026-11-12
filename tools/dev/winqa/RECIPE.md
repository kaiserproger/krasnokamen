# noVNC QA driver

Run `npm install --prefix tools/dev/winqa` once (installs `playwright-core` into the ignored `tools/dev/winqa/node_modules/`) and install a compatible Playwright Firefox. Run `node tools/dev/winqa/cs-daemon.js` as a persistent process and send one JSON object per stdin line. It prints `READY`, then a JSON response per request; every response, including errors, carries the request's `id`.

`KRASNOKAMEN_WINGUI_FIREFOX` selects the Firefox executable (default: `$HOME/.cache/ms-playwright/firefox-1555/firefox/firefox`); `KRASNOKAMEN_WINGUI_URL` selects the noVNC page (default: `http://localhost:8006/`); `KRASNOKAMEN_WINGUI_SHOTS` selects the screenshot directory (default: the ignored `.analysis/winqa-shots`, relative to the process working directory).

Requests use `{"id":"<unique>","cmd":"<command>"}`. Use `waitconn` before input; it waits for noVNC's connected state and may be used again after a VM restart. `shot` takes `tag`; `click` takes page-pixel `x` and `y`; `key` takes a Playwright key name; `type` takes `text` and optionally `delay`; `quit` closes the browser. Screenshots use a 1440×1000 viewport.

## Shift warning

The observed VNC path drops the Shift modifier: shifted characters arrive unshifted. Use lowercase and avoid shifted punctuation, including colon, double quote and underscore. In Windows, use current-drive rooted paths such as `\sc` instead of drive-prefixed paths; SMB paths are case-insensitive. For keyboard-driven dialogs, open Run, type, verify a screenshot, then press Enter. On the observed security prompt, keyboard navigation (`ArrowLeft`, then `Space`) was more reliable than mouse clicks. `cmd /k` keeps the console open for a screenshot; `cmd /c` closes it.
