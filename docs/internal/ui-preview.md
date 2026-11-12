# UI preview

Run from any Mast checkout on macOS with a Metal device:

```sh
scripts/ui-preview.sh
scripts/ui-preview.sh --themes industrial-light,industrial-dark --screens chat,details --styles default,industrial --layouts minimal,compact,guided
```

The command renders the chat shell, outline and item details, walkthrough, Settings → Appearance, and controls for every UI style. It always includes `mast-light` and `mast-dark`; `--themes` (or `MAST_PREVIEW_THEMES`) adds comma-separated built-in theme IDs. `--screens`, `--styles`, and `--layouts` restrict the selection; their environment equivalents are `MAST_PREVIEW_SCREENS` and `MAST_PREVIEW_STYLES`. Screen IDs are `chat`, `details`, `walkthrough`, `handoff`, `appearance`, `controls`, and `components`. The `components` screen shows every style/layout pair side by side in each selected theme. Style IDs are `default`, `industrial`, and `horizon`; layout IDs are `minimal`, `compact`, and `guided`. Unknown selections fail rather than silently using a fallback.

The command prints a contact sheet for each screen and an index under `$CARGO_TARGET_DIR/ui-preview/<timestamp>/` (default: the checkout's `target`). Rows are style/layout combinations, columns are themes, and each cell has a GPUI-rendered label. Sheets are capped at 4096 pixels per side. The render is offscreen and does not open an app window or capture the screen.

Set `MAST_PREVIEW_WIDTH` to a value from 320 to 4096 to render a different window width (default 1440 pixels). The height remains 900 pixels.
Set `MAST_PREVIEW_HARNESS=codex` to render the chat fixture with Codex instead of Claude Code.

Each output directory also contains `manifest.json`, listing the selected axes,
sheet paths, index path, and regular cell dimensions. Component matrix sheets
have their own layout and do not use the regular cell dimensions.

### Persistent preview server

Start `scripts/ui-preview.sh --serve /private/tmp/mast-preview` in the checkout.
Use a different request directory and Cargo target directory for each worktree.
The server owns that directory until stopped with Ctrl-C or by creating a file
named `shutdown` there. Submit one request at
a time by removing the previous `response.json`, writing JSON to a temporary
file, and renaming it to `request.json`:

```sh
python3 -c 'import json; from pathlib import Path; p=Path("/private/tmp/mast-preview"); (p/"response.json").unlink(missing_ok=True); q=p/"request.tmp"; q.write_text(json.dumps({"screens":["chat"],"styles":["default"],"layouts":["minimal"],"themes":[]})); q.replace(p/"request.json")'
```

Wait for `response.json`: it contains `manifest` and `rendererPid`, or `error`.
All four selection arrays are optional and use the one-shot defaults when
omitted; theme IDs add to the two standard themes. The server incrementally
checks the preview build on each request. It retains the renderer process and
headless windows while the executable is unchanged, and replaces the process
after a rebuild. Each window size has its own Metal renderer; the fixtures and
style globals are reset per cell. A failed or interrupted request can leave
`request.inflight.json`; submit it again as a new request after checking the
response. This is a local development protocol, with one client and no queue.

The command must run **outside the Codex sandbox**: Metal and macOS services are unavailable inside it. An enabled matching approval rule can authorize the run; otherwise Codex workers must request an escaped run. Claude Code workers and the coordinator can run it directly.

For an enabled Codex approval rule scoped to the worktrees directory, the
snapshot test forms can share `argvPrefix` `['cargo', 'test', '-p', 'mast-ui',
'--features', 'snapshot', '--test']` and `commandRegex`
`^cargo test -p mast-ui --features snapshot --test (offscreen_snapshot|ui_preview)( |$)`.
The preview script has its own `argvPrefix` `['scripts/ui-preview.sh']` rule.
The scoped matcher accepts a leading `CARGO_TARGET_DIR="$PWD/target-clean"`;
other shell expansions still require approval.

## UI changes: preview before handoff

For UI changes, use this loop before handoff (also available as the `mast-ui-render` skill):

1. Render the affected screens, styles and layouts with `scripts/ui-preview.sh --screens chat --styles default,industrial --layouts minimal`. Keep the command and selections for the comparison. Use the headless renderer only; do not launch a GUI or capture the screen.
2. Inspect the sheet and crop important cells for closer review. Rows are style/layout combinations and columns are themes; use the dimensions of the printed sheet and divide by its row and column counts. For a cell at zero-based row `R`, column `C`, with height `H` and width `W`, use `sips --cropToHeightWidth H W --cropOffset $((R * H)) $((C * W)) sheet.png --out cell.png` (replace the variables with measured pixel values). Check the crop's dimensions and labels at native resolution. On macOS, `sips` can leave the original image unchanged when an offset crop ends exactly at the bottom or right edge; reduce that dimension by one pixel and check again.
3. Compare with the merge-base using the same renderer and selections. Create a detached comparison worktree at `git merge-base HEAD origin/work-outline-implementation`, and use a **separate** `CARGO_TARGET_DIR` for it. Copy the current preview script and `crates/ui/tests/ui_preview.rs` and `crates/ui/tests/ui_preview/` into that disposable checkout so both revisions use the same renderer, then run the same command there. If those fixtures require APIs absent at the base, report the comparison gap instead of silently switching renderers. Compare corresponding crops and check for unintended changes, especially Default/Minimal.
4. Review the result yourself. Publish the relevant sheet with `artifact_publish` as an image, and include the returned `mast-artifact:` link in Ready to merge. Describe the tested selections, comparison and any visual issues or gaps.

Do not share one target directory between worktrees: Cargo can reuse or overwrite build outputs across checkouts, and concurrent builds contend on locks. Keep each checkout's target separate, including the merge-base checkout.

## Inspector spike

Build a separate development executable with `cargo build -p mast --features dev-inspector`. Mikey can launch it with `target/debug/mast` from the checkout; do not launch it alongside the installed Mast against the same live profile. Press ⌘⌥⇧I to open the inspector, hover and click an element to select it, and toggle twice to pick another. It shows the element ID and construction location; for div elements it also shows bounds and the size, padding, corner, background and border style refinements. These are specified refinements, not necessarily computed values. It is read-only and does not persist changes. This feature and interaction have not been checked in a real window.

## Rust hot-patching spike

[Subsecond 0.7.10](https://docs.rs/subsecond/0.7.10/subsecond/) can redirect calls to patched functions and lists macOS support, but its documented integration needs a patching tool and runtime patch application. Its documentation currently says only the executable's tip crate is patched, and changes to struct layout require re-instancing or discarding state. Mast's views and components live in the separate `mast-ui` library, hold GPUI entities, and use process-global style state. Therefore the documented setup does not establish that an ordinary edit to a Mast view can safely update a running preview. A useful proof would put a leaf view in the tip crate, integrate patch delivery and invalidation, and prove an offscreen before/after render while preserving compatible state. This spike has not implemented or measured that proof. Dylib reloading has the same unresolved state and code lifetime boundary: GPUI entities and callbacks must not outlive unloaded code, and Rust layouts are not a stable plugin ABI. Treat hot-patching as experimental research, not an available preview speed-up.
