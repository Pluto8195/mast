# Work Outline — implementation handoff

Prepared 2026-10-14, last updated 2026-11-06 (evening). Read this first in a new chat picking up the Work Outline work.

## Where things are

- **Repository:** `<home>/Projects/agent-mode-tools/zeron`, on GitHub as `Pluto8195/mast`
  (renamed from the `Pluto8195/zeron` fork on 2026-10-25, then `creel` → `mast` on 2026-10-26; old
  URLs redirect). `zeronsh/zeron`
  stays as the `upstream` remote, but we no longer open PRs there. On 2026-11-09 the repo left GitHub's fork
  network: it's standalone and **public**, and it can now be made private. Porting from upstream works by fetching
  the `upstream` remote; triage is parked (item 07486a45). The folder and submodule path
  are still `zeron/`, and so are all code identifiers.
- **Integration branch:** `work-outline-implementation`, branched from `main` at `3801cb5b`. It is
  the GitHub default branch. `main` is upstream's code at the fork point. **Never commit to
  `main`.** `origin` carries only these two branches; delete each feature branch once it is merged.
- **Parent repo:** `agent-mode-tools` branch `zeron-work-outline` pins the `zeron` submodule.
  Restore points: tag `pre-linear-integration` (`83f49b3d`) and parent commit `d9129f6`.
- **Original spec:** `work-outline-implementation-specification.md` (copy of Mikey's spec, plus an
  "Implementation contract amendments" section). The binding contract is `m0-contract.md` in this
  folder; it wins where it differs from the spec.
- **Installed build:** `/Applications/Mast.app` (since 2026-10-26). Since 2026-11-05 its bundle id is
  `computer.mikey.mast.dev` and its executable `Contents/MacOS/mast` (`zeron` is a symlink for one release); data
  stays in `~/.zeron` (new installs use `~/.mast`). Before that it shared `sh.zeron.app.dev` with the earlier
  `Creel.app` and `Zeron Dev.app`, which are
  rollbacks only (`Zeron Dev.app` is in `~/zeron-dev-app-backups/`); never run two at once. Rollback binaries are
  in `~/zeron-dev-app-backups/`, one per install, named after the feature. Keep only the last few.

## Built and installed

- **Release 94: menu-bar redraw fix** (2026-11-16; UI 2646/0). menu_bar::MenuSync sets the status-item icon only when the
  glyph changes and the tooltip only when its text changes, and rebuilds the NSMenu only in menuWillOpen: or while the
  menu is open (menuDidClose: tracks open/closed). Before, a full menu rebuild about every 900 ms took ≈40% of the main
  thread. Backup: `~/zeron-dev-app-backups/mast-release-93`.
- **Release 93: startup watch-storm fix** (2026-11-16, urgent; diff_sync tests 20/20, worker's full mast-engine 711/1
  with the 1 the known instance_lock flake; installed build only, no full release run because of the load emergency).
  After each restart Mast pinned about 8 cores for many minutes: every diff-sync entry walked up to 8000 dirs in
  parallel. With ~116 Cofactr worktrees (node_modules) plus Mast worktrees (target/), that froze or lagged the app. Now
  at most 2 watch setups run at once, trees with node_modules/target/.venv/dist/build/.next/vendor are over budget
  immediately (git-dir watch only), and verdicts are cached per root. Still open:
  - The menu-bar item redraws about every 900 ms (≈40% of the main thread in -[NSStatusItem _updateReplicants]); being
    fixed on the same branch.
  - repos/source_control git fan-out (`git branch -r --contains` across Cofactr worktrees).
  Backup: `~/zeron-dev-app-backups/mast-release-92`.
- **Release 92: composer crash fix** (2026-11-16; touched crates from 6b24713d4: all passed; UI 2643/0). Release 90
  aborted in `ComposerInput::drag_selection_position` during a drag-select. `f32::clamp` panics when min > max, and a
  text box under 0.5px made the upper limit fall below the lower one. That clamp and two others in the composer that
  have variable bounds are now guarded; test `collapsed_input_bounds_do_not_panic_during_drag`. Rule for workers:
  every variable-bound `clamp` must guarantee min <= max. Backup: `~/zeron-dev-app-backups/mast-release-91`.
- **Release 91: active checkout P0 + review tool schemas** (2026-11-16; full suite from bd0aac02a: nextest 2605/2607,
  only the known `peer::actual_webrtc_pair_streams…` and `preview::leak remote_preview_churn`; UI 2642/0).
  - Active checkout P0: when a chat has a selected diff target (bind_chat_worktree or a manual pick), the header
    branch chip, "Runs in" (execution root shown as secondary), Item tab, sidebar row, Overview details and search,
    command palette, pickers, and the PR lookup and watch keys all follow that checkout. An explicit checkout skips the
    `.gitmodules` same-branch fallback; `is_worktree`/`diff_stat` come from the checkout. P1+ (inference, touched
    sets, PR by remote) shelved by Mikey; re-measure first.
  - review_publish / review_get: flat input schemas (no anyOf/oneOf/allOf), one target object with a `kind` enum,
    example in the description, errors that name the field path (`review.summary`, `target.number`), and
    `review.target` rejected in favour of the top-level target. From failed-tool-call audit pass 2
    (mast-artifact:ff5aaaac), where 22 of 35 failures were review_publish retries. Fixes 2–5 wait for a re-run on a
    larger sample (item ca2d95cc waits until 2026-11-23).
  - Backup: `~/zeron-dev-app-backups/mast-release-90`.
- **Release 90: review follow-ups** (2026-11-14; touched crates from ded3ec8f9: all passed; UI 2639/0): Claude
  limit-error dedupe resets at turn boundaries; admin reach skips archived chats except `archive_chat` (unarchive);
  decision-sheet tables wrap inside their cards at narrow widths (verify visually after restart).
  Backup: `~/zeron-dev-app-backups/mast-release-89`.

- **Release 89: coordinator reach + approach options** (2026-11-14; tests for touched crates from fc0b007b6: nextest
  2545/2547, only the known `peer::actual_webrtc_pair_streams…` and `preview::leak remote_preview_churn`; UI 2639/0).
  - Coordinator reach: send_message, read_chat and wait_for_turn reach non-archived chats in the caller's item subtree
    or project, besides its own children; a worker always reaches its parent. respond_to_input and interrupt stay
    children-only (no worker answers another chat's approvals). There's no self-messaging. Sends to non-child,
    non-parent chats are refused if the target has a wider sandbox or auto-approve. Last-sender reach is disabled:
    the relayed header is spoofable, and structured sender provenance is a follow-up. Sheet and plan answers route to
    the latest-revision publisher, else the first publisher. The invalid-link error shows an example.
  - Approach options: decision-sheet rows take `options` (2–3, `recommendedOption`); options render as neutral
    selectable cards with a Recommended marker and stage like Accept; decisions carry `optionId`/`label`; a missing
    decision date is hidden. The coordinator fixed a missed `SheetDecision` initializer in a mast-doc test.
  - Follow-ups: admin reach still includes archived chats; the project-planner skill should start emitting `options`
    on approach rows.
  - Backup: `~/zeron-dev-app-backups/mast-release-88`.

- **Release 88: workflow pack + agent logging/error fixes** (2026-11-13, 3af296e43; full suite from 4a08b739d:
  nextest 2589/2590, only the known `preview::leak remote_preview_churn` hang; UI 2637/0).
  - Workflow pack: Settings → Workflow pack exports a Space's instructions and templates and imports them elsewhere,
    with a per-conflict Keep mine / Add missing / Replace preview, single-lock apply and undo from receipts stored
    host-locally in `workflow-pack-receipts/`. Admin-only MCP tools `work_outline_workflow_pack_*`; format in
    `docs/workflow-pack-format.md`.
  - Logging, from the agent error audit (mast-artifact:7e446c29): a `ts` on every journal record; `exitCode` on
    ToolResult; Codex MCP failures keep their error text (failures only); `chatId`/`spaceId` on sessionStarted;
    usage-limit errors carry `code` and `resetsAt` (ms/s sanity, 8-day cap), and auto-wakes and auto-continue defer
    until reset (a deferred auto-continue is lost on restart; Errored workers auto-continue after reset); 7 daily app
    log files, legacy `zeron-*.log` removed; friction reports keep per-occurrence history (last 50).
  - Follow-up: after a rejected `rate_limit_event` with no `result` frame, `emitted_limit_error` stays set, so the next
    turn's limit is missed.
  - Backup: `~/zeron-dev-app-backups/mast-release-87`.

- **Release 87: worker Changes in one click + GitHub-style file tree** (2026-11-13, 4c4dc0d61; UI 2634/0, mast-gallery
  8/8): Team, Item and running-chat worker rows get a "Changes" link (worktree first, pushed-branch fallback; was 3
  clicks); Ready to merge diff links use the same path. Changes file tree: collapsible folders with per-diff state,
  collapse/expand-all icon buttons, compacted single-child paths, file icons, counts on collapsed folders, ←/→ on a
  focused folder, draggable persisted width. The coordinator moved the Changes link into
  `shell_parts::worker_action_link` to keep the component-lint text-size ratchet at 410.
  Backup: `~/zeron-dev-app-backups/mast-release-86`.

- **Release 86: Needs-you cards in place + smaller coordinator reads** (2026-11-12, eeb144b88; tests for touched
  crates from f45e1be44: nextest 2500/2502, only the known `peer::actual_webrtc_pair_streams…` and
  `preview::leak remote_preview_churn` hang; UI 2630/0): a decision sheet's or plan's Needs-you card is keyed on the
  artifact and updated in place across revisions (a resolved sheet card only reopens on a newer revision; plan
  questions fold into the one plan card; legacy per-question cards keep working); agents update their own open request
  through the `dedupeKey` path; a chat that read `~/.agents/skills/*/SKILL.md` or `~/.claude/skills/*/SKILL.md` gets
  one "<name> skill changed since you loaded it" line on its next message (not on `/` commands);
  `work_outline_read_subtree` omits completed items unless they hold open requests or live chats (`includeCompleted`,
  `completedOmitted`; 46 KB → 1.2 KB on a 173-item fixture); `report_to_parent` / `create_chat` suggest STATUS /
  RESULT / NEXT reports. Backup: `~/zeron-dev-app-backups/mast-release-85`.

- **Release 85: PR #1 (David Wallach) idle CPU + input latency** (2026-11-12, 52ab0bcbb; tests for touched crates
  from 638cf1008: nextest 1327/1328, only the known Tailscale-only `peer::actual_webrtc_pair_streams…`; UI 2630/0):
  search index re-walks on repair or rate-limited picker open, idle indexes retire; diff sync ignores foreign git-dir
  churn but tracks (nested) submodule refs; preview scan backs off to 10 s; sidebar culls far-off rows; composer and
  pickers redraw less. Backup: `~/zeron-dev-app-backups/mast-release-84`.

- **Release 84: visual audit fixes** (2026-11-12, aa47417fe; full suite from 07ecc9648: nextest 2534/2535, only the
  known `preview::leak remote_preview_churn` hang; UI 2625/0): decision sheet flow lanes wrap/scroll with attached
  connectors; row controls (Accept primary, others secondary, monochrome note button, "Your answer" placeholder);
  Horizon light status dots per state; walkthrough content width; distinct Appearance style tiles; `handoff`
  ui-preview screen; `artifact_render_check` renders decision-sheet/plan/options JSON by path (optional `kind`).
  Backup: `~/zeron-dev-app-backups/mast-release-83`.

- **Release 83: artifact export** (2026-11-12, 6379a3c2c; targeted: mast-proto 282, mast-ui artifacts 239): an Export
  menu in the artifact tab header: standalone HTML (kit CSS inlined, Mermaid from the shared CDN, decision sheets
  read-only), A4 PDF via offscreen WebKit print (not testable headless; Mikey to try a long plan), Copy as markdown
  for plans, decision sheets and markdown. Backup: `~/zeron-dev-app-backups/mast-release-82`.

- **Release 82: structured plan artifacts** (2026-11-11, 2ecae0d4b; targeted: mast-proto 279, mast-engine
  artifacts 43 + agent_tools 62, mast-mcp 55, mast-ui artifacts 235):
  - New artifact kind `plan` (.json): crates/proto/src/plan.rs (types, strict parse, the 8 rules in
    docs/artifact-kit/plan-schema.md incl. linked-sheet row checks, `linear_body`) and plan_render.rs (summary chips,
    build-order strip, ticket cards beside their Linear body, citation chips, collapsed evidence). Example:
    docs/artifact-kit/plan-example.json (synthetic; the real kit plan stays private). `artifact_get_plan` returns the
    JSON for plan-kind artifacts; markdown plans unchanged.
  - Backup: `~/zeron-dev-app-backups/mast-release-81`.

- **Release 81: render check false positives** (2026-11-11, f5a2c6f98; fast-tracked, mast-ui artifacts 234/0): the
  contrast check falls back to the rendered theme's `--background`, not the OS dark-mode query; `narrowTextColumn`
  skips empty text, `pre` and cells of tables that fit. Backup: `~/zeron-dev-app-backups/mast-release-80`.

- **Release 80: render check works again** (2026-11-11, d22a2145f; fast-tracked, mast-ui artifacts 234/0):
  - The in-frame render probe referenced an undefined `metrics` since e8ff10cb4, so `report()` threw and every
    `artifact_render_check` said "The render probe did not report" (bb6e6f10). The probe now collects
    `ElementMetrics` (capped at 1,000) for the layout heuristics, and any probe exception still posts a report with
    "Render probe failed: …". Backup: `~/zeron-dev-app-backups/mast-release-79`.

- **Release 79: hand off to another provider/model** (2026-11-11, 07ecc9648; full suite):
  - Hand off to new chat (dialog, `HandOffOrchestrator` RPC, `hand_off_orchestrator` MCP) takes optional
    `harness`, `model`, `reasoning`. The dialog reuses the new-chat picker, preset to the source chat. A different
    harness starts from its default model; sandbox and Ask/Auto carry over; models are checked against the
    harness catalog (`hand_off_config` in crates/engine/src/rpc.rs).
  - Tests (`run_release_tests.sh bd9f573d6`, covering fast-tracked 77/78): nextest 2469/2471 (the two known
    WebRTC preview tests; crate unchanged), UI 2618/0. Backup: `~/zeron-dev-app-backups/mast-release-78`.

- **Release 78: decision-sheet keys + row outcomes** (2026-11-11, 14ef72b3f; fast-tracked, targeted tests only):
  - Artifact frames report edit focus (`mastEditFocus`), so a focused note/answer field gets bare-key shortcuts
    (e/n/p/t/v) and ⌘A/C/V/X/Z. Cmd/Ctrl app shortcuts and browser keys still go to Mast; Esc in a field returns
    focus to the app (crates/ui/src/browser/macos.rs `key_route`, `FIELD_EDIT_KEYS`).
  - Rows offer Accept / Answer differently / Don't do this. "Don't do this" (`rejected`) only on `optional: true`
    rows, enforced by the engine; answers are `decided` + `overrideRecommendation` with the text in `answer` from
    artifact_get_decisions. Contract: docs/artifact-kit/decision-sheet-schema.md.
  - Tests: mast-proto 274, mast-ui artifacts 232 + browser 39, mast-engine `--test artifacts` 41. Real WebKit typing
    not yet confirmed. Backup: `~/zeron-dev-app-backups/mast-release-77`.

- **Release 74: data-driven decision sheets + responsive kit** (2026-11-10, a68e8d99d):
  - `decision-sheet` artifact kind: strict JSON validation, a kit renderer, computed chips, a dedicated 1024 px wide
    host (other fragments keep their cap), compare. Child status pills show only when they differ from the parent.
    Sample: docs/artifact-kit/decision-sheet-example.json. Phase 2 (row ✓/✗/💬 and one Needs-you per sheet) is
    item 3b9a83cb.
  - Kit: lanes stack under the container breakpoint (700 px; `mk-lane-dense` at 900 px), tables scroll in their
    card, grids drop to one column.
  - Tests: nextest 2455/2457 (the two WebRTC preview tests, `peer::actual_webrtc_pair_streams_in_both_directions`
    and `leak::remote_preview_churn…`, fail only in this Mac's Tailscale environment; the crate is unchanged since
    release 70), UI 2609/0. Backup: `~/zeron-dev-app-backups/mast-release-73`.
  - The release ran detached from Mast (python setsid wrapper, /tmp/release74-run.sh), so restarts and
    force-quits can't kill it. Use this for future releases.
  - Release 73 froze once after a chat-row click (bug 17c9d278; next time `sample` before quitting).
  - Skills: `talk-to-me` is now one shared copy in `~/.agents/skills` (linked for claude/codex/cursor), with an
    "LLM writing tics: never" table that applies to all writing, including design and UI text.
- **Release 73: PR reviews never move the main checkout** (2026-11-10, 3de8e23a5):
  - `choose_location` no longer runs `gh pr checkout` in a clean canonical checkout. It reuses an existing
    checkout of the branch, or creates `<root>/<repo>-pr-<n>`.
  - Review chats still start at the Space root, so workspace skills resolve. The prompts confine work to the
    checkout and forbid branch switches elsewhere. The diff binding uses the checkout.
  - Tests: nextest 1296/1296, UI 2608/0. Backup: `~/zeron-dev-app-backups/mast-release-72`.
  - Skills, outside the repo, now single copies in `~/.agents/skills`, symlinked into `~/.claude` and `~/.codex`:
    - new `review-pr`: a safe review checkout, then the repo's `code-review`
    - global `code-review`: routes named PRs to `review-pr`
    - `~/.codex/skills/implement-ticket` synced with `~/.claude` (still two copies)
  - The Cofactr Space-root lines (review-pr + quiet orchestrator) await Mikey's paste. Admin grants only reach
    the chat's own Space.
- **Release 72: open-file limit, log fixes, artifact kit** (2026-11-10, a9fb7d41b):
  - The app raises its own RLIMIT_NOFILE (256 → min(hard, 8192)) at launch and logs it. Native watchers share a
    budget, with repair polling as the fallback.
  - Window updates from app actions are deferred; this targets "RefCell already borrowed", though the cause isn't
    confirmed.
  - Model-discovery failures keep their cause; wake pruning logs at debug.
  - `mk-` artifact component kit (`crates/ui/assets/artifact-kit.css`) injected into every fragment host, plus
    docs/artifact-kit/ (gallery, decision-sheet example, JSON schema) and the conventions doc.
  - Tests: nextest 1296/1296, UI 2608/0. Backup: `~/zeron-dev-app-backups/mast-release-71`.
  - Skills edited the same day, outside the repo (backups in `~/zeron-dev-app-backups/skills-2026-11-10/`):
    - `implement-ticket`: orchestrated mode (decision-sheet rows only, push + draft PR without asking, one "Ready
      for review" gate, screenshots on UI PRs).
    - `project-planner` §2a: a decision sheet is required before tickets.
  - Bug filed: artifact_render_check always reports "probe did not report" (bb6e6f10).
- **Release 71: perf, Retry, Codex sign-in** (2026-11-10, 289bb5688):
  - Perf:
    - Loro 1.13 → 1.17: an upstream snapshot-growth fix (streaming test 1.13 GB → 2.8 MB; legacy 1.13.7 fixtures load).
    - diff-sync ignores checkout read events, which stops a git feedback loop.
    - The orchestrator-wake timer skips outline materialization when nothing is due.
    - PR watches use a known repo root.
    - Sidebar skips layout for off-screen/closed rows, and the heartbeat no longer redraws the sidebar (400 chats:
      10 frames 330 → 129 ms; previews byte-identical).
  - Retry on the latest errored turn and failed composer sends.
  - A signed-out Codex shows "Authentication failed — sign in to Codex again."
  - "Charting"/"Masting" working words; first-run regression tests; README Updating section.
  - Tests: nextest 2502/2503; UI 2608/0. The one failure is `mast-preview::leak
    remote_preview_churn_does_not_accumulate_tasks_or_memory`, which hangs even at 3 iterations. Its crate and deps
    are unchanged since release 70, so it's likely ICE over the Tailscale utun interfaces; quarantine it if it
    recurs.
  - Pre-release backups: `~/zeron-dev-app-backups/pre-release-71-2026-11-10/` (docs.sqlite3 read-only copy,
    mast-release-70).
  - Baseline before restart: 4.4 GB RSS fresh, 5.4–6.4 GB after 2 days, UI thread 94% busy. Profiles are in
    `~/zeron-dev-app-backups/perf/`. Follow-ups: item 2694931e.
- **Release 70: Horizon polish + Linear/team migration** (2026-11-08): Horizon SEND is muted on any theme; Horizon
  selected Appearance cards are marked; Linear and team screens are on shared components (all planned screen
  migrations done).
- **Release 69: Horizon** (2026-11-07): the Area style is renamed Horizon; saved `area` settings, themes and
  style overrides still load. Horizon list bullets follow the agent colour.
- **Release 68: Area tweaks** (2026-11-07): Area replies wrap at a reading width with quiet bullets; agent colour
  follows the provider (Claude amber, Codex green); Area outline uses rounded nesting; composer menus are
  style-aware. Details in the 23:10 "In flight" snapshot.
- **Claude Default reauthentication** (2026-11-07): signing in again to the active Claude account updates the live OAuth credentials before the account list snapshots them, preserving shared credentials. Distinct accounts remain saved separately. Release suite passed (2,480 non-UI and 2,581 UI tests); installed build has not yet been verified with a real login. Use Settings → Agent Accounts → Add Claude account, then retry the existing chat after restarting Mast.

All milestones M0–M4 of the spec, then the follow-ups below. User docs: `docs/work-outline.md`,
`docs/pr-review.md`.

- **Outline (M1):** durable work items in the profile registry (row table, HLC last-writer-wins per
  field, 16 KiB op cap), Spaces as roots, chats under items, Canvas positions preserved.
- **Instructions (M2):** per-item instructions with revisions. Claude Code gets them via
  `--append-system-prompt-file` + `--system-prompt-snapshot off` on every spawn; Codex via
  `developerInstructions` with a fresh session on change (or "keep session"). Cursor/ACP refuse.
- **Execution (M3):** per-chat execution bindings, multi-repo roots, worktree assignments.
- **Required Actions and agent tools (M4):** `zeron-agent` MCP server with a per-Run token;
  `work_outline_*` tools; Reply & resolve sends the answer to the chat.
- **Keyboard and editing:** Tod-style keys (↑↓ in visual order, ⇧arrows move/indent, Tab/⇧Tab,
  F2/R rename incl. chats), drag and drop, right-click menus that show their keys, `?` reference,
  ⌘; outline⇄chat, ⇧⌘A archive, ⌘↩ Start Chat, ⇧⌘↩ Open Full Chat, ⌘F search, preview on select,
  Space root reorder/hide/rename/delete, tree guide lines, Undo toast for moves.
- **Latency fix:** the local profile never acked pending registry batches (67k queued); now
  `settle_pending_locally` when there is no edge, plus optimistic UI.
- **Editors:** skill autocomplete (always Claude Code + on-disk skills, regardless of the chat's
  harness), auto-growing instruction editors.
- **Review PR launcher (⇧⌘P):** matches the Cofactr workspace `launch.sh` flow.
- **Linear v1** (`linear-contract.md`): per-Space opt-in, key in keychain `sh.zeron.linear`, Inbox
  column on the left (⌥⌘L) with collapsible Issues/Projects. Nodes are created **manually only**
  (Mikey: "I don't want things changing out from under me"). Title/description follow Linear until
  edited; the full ticket goes to agents; project-linked item is the issue destination; "Add
  issues from project". Nothing is written to Linear. GraphQL is **not yet verified against the
  real API** (projects and attachments especially).
- **PR review sync** (`pr-sync-contract.md`): automatic items under one chosen destination item
  per Space (covers submodule repos), Required Actions "Review requested" / "Re-review
  requested", auto-archive on merge/close. PR links on ticket items come from Linear attachments
  first, chat-created PRs as fallback.
- **Planning artifacts phase 1** (`../planning-artifacts-design.md`, `../artifacts-contract.md`,
  agent conventions `docs/planning-artifact-conventions.md` v1.1):
  - Stored in `<canonical checkout>/.zeron/artifacts/<work item>/`, ignored via
    `.git/info/exclude` (never `.gitignore`); snapshots under `<profile>/artifacts/<id>/`.
  - `artifact_publish` / `artifact_get_plan` / `artifact_render_check` agent tools.
  - Render check uses an offscreen WKWebView snapshot (not screen capture), viewports × themes up
    to 4 images, and layout heuristics.
  - Links inside rendered artifacts route through Zeron. Files outside the workspace get "Open as
    artifact" (credential paths, `.env*`, and files over 10 MB are refused).
- **Visual compare for artifact revisions** (`crates/ui/src/artifacts/{compare,pixel_diff,
  markdown_diff}.rs`). Mikey's complaint: text diffs of charts are useless.
  - HTML, Mermaid and images: two revisions side by side with synced scroll. "Highlight changes"
    runs a pixel diff with region boxes; there is also onion skin and a Source tab for the line
    diff.
  - Markdown: rendered side by side, changed blocks tinted, section summary on top.
  - Options: compares two options of the current revision.
  - Verified only by headless tests.

- **Git uses the login-shell PATH** (2026-10-19): every engine `git` spawn goes through
  `zeron_harness::git_command()` / `git_command_async()`. Before this, worktrees in LFS repos
  (flagship) failed with "git-lfs: command not found" because the GUI app's PATH lacks
  `/opt/homebrew/bin`. New `git` spawns must use these helpers.
- **Error notices** (`crates/ui/src/error_notice.rs`): errors are a red card with full wrapped
  detail and a Copy button, and stay until dismissed. Used by the PR review launcher, Start Chat,
  composer send/worktree failures, and error toasts.

- **Coordinator agent tools** (2026-10-20): `create_chat { workItemId, prompt, … }` starts a
  child on the caller's item or a descendant (same launch path as Start Chat, so the target's
  instructions and execution binding apply; `cwd`/`branch` refused). `work_outline_read_subtree`
  lists the items, chats, and open Required Actions under the caller's item. Children cannot message
  their parent; coordinators collect results with `wait_for_turn` / `read_chat` / Required
  Actions.
  In the outline a child chat nests under its parent chat only when it has no item of its own or
  shares the parent's item; a coordinator's child shows under the item it was started on.

- **Outline fixes** (2026-10-20, first release by the coordinator):
  - Instructions longer than 4 lines collapse behind Show more / Show less. ↑ ↓ Edit Remove sit in
    the entry's header row. The instruction action "Archive" is now "Remove" and still has undo.
  - Synced PR and Linear chips keep their last data during a refresh or after a failed pull. They
    used to drop to "#42" / "ENG-1" on every 2-minute (PR) or 5-minute (Linear) refresh. A
    search-list row no longer overwrites a fully fetched PR status (`github/store.rs`,
    `linear/store.rs`).
  - An archived item with no visible child item is hidden even when it still has live chats; those
    chats show under Unassociated Chats. This used to leave archived synced PR items on screen,
    dimmed and immovable, because they keep their review chat (`work_outline/model.rs`).
- **Coordinator resolves its children's requests** (2026-10-20): `resolveUserActionRequest` is
  allowed when the request's origin Chat is the caller or a Chat whose `parentChatId` is the
  caller. This covers direct children only, not grandchildren or chats a person started
  (`m0-contract.md` § coordinator amendments, item 3).
- **Visual compare follow-ups** (2026-10-20):
  - The per-kind compare mode persists in `UiSettings` (`artifactCompareModes`).
  - Markdown panes scroll together by block position. Wheel and trackpad sync; dragging the
    scrollbar does not.
  - Highlight changes and onion skin cover the full render, up to 8000 CSS px and 20 M px per
    snapshot. Taller content is cut off silently. The agent render check is unchanged.
  - Options compare can follow one option across two revisions. The engine snapshots option files
    per revision, and `ArtifactContent` serves them by revision. Revisions recorded before this
    show "not found" until the artifact is republished.
  - The tall-snapshot path has only run in headless tests, never against a real WKWebView.
- **Plan reading: ⌘F find and section index** (2026-10-20; `artifacts/{find,outline}.rs`,
  `artifacts/tab/find_view.rs`, `files/markdown_find.rs`):
  - ⌘F opens a find bar while focus is in an Artifact tab. It shows a match count and highlights
    every match. Enter / ⇧Enter and ⌘G / ⇧⌘G step through matches with wraparound, and Esc closes
    the bar. Outside an Artifact tab, the shell's ⌘F and ⇧⌘G handlers `propagate()` when their own
    route doesn't apply. ⌘F is still outline search everywhere else.
  - A collapsible section rail lists h1–h3 headings: click to jump, and the section in view is
    highlighted. It's hidden under 3 headings and for options, source, compare, images and Mermaid.
  - HTML find runs over postMessage into the sandboxed frame, and WKWebView forwards ⌘F, ⌘G, ⇧⌘G
    and Esc to the app. Highlights need macOS 14.2+. Matches that span inline tags aren't found.
    Find and the index are off for HTML on Linux.
  - Only headless tests have run. Key forwarding, the frame script under the CSP, and active-section
    tracking haven't been seen in a real WKWebView.
- **Plan section keys** (2026-10-20):
  - With the section rail focused (click it, or press ⌥⌘S), ↑ / ↓ / Home / End move through
    sections and scroll the document. Enter or Esc returns focus to the document.
  - ⌥↑ / ⌥↓ anywhere in the plan jump to the previous / next section, with no wraparound. These keys
    are bound only in the ArtifactTab key context.
  - `browser/macos.rs` now maps AppKit's special-key characters (arrows, Home / End, Esc) to
    combos. Before this, Esc forwarding from WKWebView probably never worked.
  - The `?` reference doesn't list these keys yet.
- **Compact icon PR / Linear rows** (2026-10-20, `github/row_icons.rs`):
  - PR rows end in fixed slots, in this order: author, needs (attention dot + count), your review
    (eye / refresh / check / close-circle), CI dot, and the PR icon colored by state. Hovering the
    group shows the full line, the details panel keeps the full text, and `?` has a legend.
  - On narrow screens the slots drop CI, then review, then author. The title keeps about 120 px.
  - "You" for the viewer's own PRs isn't shown, because the UI doesn't know the viewer's GitHub
    login (only the engine does, via `gh api user`). Adding it needs an RPC.
  - Items with several PRs show icons only for the first one.
  - Linear rows get a state dot plus a status icon, but keep their totals text and don't drop slots.
- **Question cards** (2026-10-20; first slice of Artifacts phase 3 D, rendering only):
  - `markdown/question.rs` `parse_question` turns a top-level `> [!question] Q-n · …` quote into
    `Block::Question`. `markdown_diff` reads ids from it, and its old private parser is removed.
  - Each card shows an id badge, a status pill, a Blocking flag, the question in bold, and Example /
    Blocks / unknown fields / Decision rows. Decision is tinted. Resolved cards are dimmed and
    collapsed, with a Show/Hide toggle, and expand while find is active.
  - Transcripts render question cards too.
  - The HTML `data-zeron-question` form gets CSS plus a frame script (`artifacts/question_html.rs`).
    It's only been checked headlessly, never in a real WKWebView.
  - Still to do for D: the Questions panel, Required Actions for questions, and answering from Zeron.
- **Diff the agent's worktree** (2026-10-21; `m0-contract.md` "Amendment: chat diff targets"):
  - Cofactr chats start at the workspace root, and agents then `git worktree add` into a gitignored
    `.worktrees/<submodule>/<slug>`, so the Changes pane used to show the root's unrelated diff.
  - A new mutable registry row, `chatDiffTargets`, is written by two things: the agent tool
    `bind_chat_worktree {path}` (validated against the chat repository's worktree family), and the
    Changes pane's "Diff checkout…" picker, which suggests ticket matches first.
  - The selected target drives the Working tree, Branch and Latest turn scopes, as well as the member
    pills and the composer branch label. It is display-only and never blocks a turn; execution
    bindings were deliberately not reused.
  - Not done: remote-device chats, PR lookup from a target, and automatic binding without the agent.
    If it is added later, tag auto binds `source: detected` and never take the selection away from
    a manual pick.
- **zeron-agent MCP instructions fit the truncation limit** (2026-10-21):
  - Claude Code cuts MCP server instructions at about 2000 characters. `INSTRUCTIONS` in
    `crates/mcp/src/jsonrpc.rs` was about 2500, so Claude agents never saw the Planning artifacts
    paragraph.
  - It is now 1886 characters, with a test enforcing ≤ 1900 and checking the key phrases. It tells
    agents to call `bind_chat_worktree` right after `git worktree add`.
  - Keep new guidance short and near the top, or put it in the tool descriptions.
- **Work-context strip in the full chat view** (2026-10-23, `chat_work_context{,/strip}.rs`,
  `shell/work_context_ui.rs`):
  - It's one line, expandable. Variants by precedence: PR review item (own item, ancestor, or a
    launcher review chat) > Linear issue (the ticket plus its PRs, or "No PR yet") > Linear project
    or pinned plan (plan title, revision, age, "+N artifacts", or "No plan yet"). No strip otherwise.
  - Launcher review chats are detected by their `review_launcher` chat-PR link
    (`ChatPr::launched_review` in `chat_work_context.rs`).
  - Follow-ups: PR line counts / unresolved threads / conflicts; open-question counts in the
    planning strip; matching the chat's diff-target branch to PR heads; the strip in the overview's
    chat panel.
- **Coordinator resolve fix** (2026-10-23):
  - Bug: coordinator resolves of a child's request silently wrote nothing when the request had a
    `dedupeKey`, which agents always pass. The dedupe group was filtered down to the caller's own
    requests, yet the tool still replied `resolved:true`.
  - Fix: `agent_may_resolve_origin` (the caller's own chat or a direct child) is shared by
    authorization and the group filter. The tool reads the row back and returns `operationConflict`
    if nothing changed.
- **PR page parity with pr-dash** (2026-10-23; `proto/src/pr_dash.rs`, `engine/src/pr_dash.rs`,
  `engine/src/pr_chat_links.rs`, `ui/src/pr_page/`):
  - ⌘P (`OpenMyPrs`) and ⇧⌘P open the PR page. It has pr-dash's two tabs: Authored, and Review
    requested (`review-requested:@me` + `reviewed-by:@me -author:@me`), fetched through the
    `PrDashboard` GraphQL RPC. The 9 authored and 8 incoming bucket rules are ported, with
    needs-you counts and churn, reviewer, CI, thread and conflict fields. The old chat table is
    kept as a third "Review chats" tab.
  - Chat-PR links gained the sources `branch`, `diff_target`, `ticket` and `review_launcher`. URLs
    are compared in canonical form, and every PR URL from one `gh pr create` is linked. The
    launcher reuses an existing chat for the same PR and action. A launcher review chat is
    identified only by its `review_launcher` link.
  - The old overview "My PRs" sidebar is unreachable; deleting it is a follow-up after
    remove-canvas.
  - Not done: the leaderboard tab, the person picker, the churn discount for blank and comment
    lines, the diff bar, and file paging beyond 100 files.
- **Diff a branch or PR without a checkout** (2026-10-23; `m0-contract.md` "Amendment: diff a
  branch or PR without a checkout"):
  - "Diff branch / PR…" sits next to "Diff checkout…". It suggests linked PRs first, then
    ticket-matching branches, and accepts a PR number, URL or branch name as free text.
  - The repo is picked from the PR's owner/repo across the checkout and its submodules. Refs are
    fetched read-only, falling back to `refs/pull/<n>/head`. The base is explicit, else the PR's
    base, else `origin/main`.
  - The diff is `merge-base...head`, via the `range` mode of `GetCheckoutDiff` /
    `GetCheckoutFileDiffText`.
  - Stored as `refTargets` / `selectedRef` on the `chatDiffTargets` row.
  - Not done: auto-refetch, and an unbind control in the UI.
- **Row icon follow-ups** (2026-10-23):
  - A `PrViewer` RPC returns the cached `gh api user` login, so rows and the chat strip show "you"
    for your own PRs.
  - Linear rows are icon clusters (assignee · needs · status) with narrow-width dropping, and their
    totals text moved to the tooltip.
  - `?` has a Plan group and "Open the PR page".
  - Items with several PRs show up to 3 PR icons plus "+N".
- **Canvas view and chat auto-classification removed** (2026-10-23; restore point: tag
  `pre-canvas-removal` at `8bdce21d`; 44 files, −10,869 / +1,278):
  - `OverviewMode` is List / Outline. A saved `viewMode:canvas` loads as Outline.
  - Classification is gone: the RPCs, the classifier, the startup repair and reclassify passes, the
    header category/origin chips and override menu, and Reclassify.
  - List grouping is Repo / Ticket. Expanded List rows lost "launched via" and the tools/skills
    tallies.
  - The generic TypeSafe/Jev client in `typesafe.rs` is kept for reuse.
  - The PR page "Review chats" tab lists chats with a `review_launcher` link when no destination is
    set.
  - Stored classification and position files are left on disk, unread.
- **Forgiving agent tools** (2026-10-23; `agent_tools::normalize_arguments`, `proto/src/agent.rs`):
  - Agents often call zeron-agent tools before their schema loads. These tools now derive an
    idempotency key when it's omitted: `bind_chat_worktree`, `set_status`,
    `associate_current_chat`, `resolve_user_action_request`, and `create_user_action_request`
    when it has a `dedupeKey`.
  - Aliases are accepted: text/title/question → request; id/requestId → userActionRequestId;
    itemId/workItem → workItemId; snake_case → camelCase.
  - Argument errors list the tool's parameters with an example call.
  - Keys stay required for create_chat, create_child, propose_instruction, artifact_publish and
    create_user_action_request without a `dedupeKey`.
- **Review chat replaces Review; My PRs sidebar deleted** (2026-10-23; `ui/src/review_chats.rs`):
  - A PR's review chat is a live chat assigned to its review item, or one linked with source
    `manual` or `review_launcher`. When one exists, the PR page row's primary action is "Open
    review" (title plus live state), and Review moves to the row menu as "Start another review".
  - Enter on a PR row opens the review chat, else starts a review; ↑↓ move the row highlight.
  - The same "being reviewed" marker shows on the outline row and in the chat strip.
  - The overview "My PRs" sidebar code is deleted; old UI-flag keys still load.
  - Authored rows get the same Review / Open review actions, with Fix CI kept when CI fails. When
    the viewer authored the PR (the cached `gh api user` login matches the `gh pr view` author),
    the launcher seeds a self-review prompt; otherwise it keeps the reviewer prompt.
- **Links open in Zeron's side browser** (2026-10-23; Shell `open_web_reference`, pure `reference_open`):
  - With `openWebLinksInZeron` on, every web and PR link (PR page, outline icons, chat strip, chips,
    transcript and artifact links) opens in the side browser. A tab already on that page is focused
    instead of opening a duplicate.
  - The side pane exists only on the Chat route. From the PR page, Overview or Outline, a link
    switches to the PR's review chat, else the selected chat, with the browser beside it; with
    neither, it opens externally.
  - ⌘-click, or "Open in External Browser" in the outline PR menu, always opens externally.
- **Renamed to Creel, user-facing** (2026-10-23; decisions under Parked → "Rename the Zeron fork"):
  - Name: about 190 visible strings, both plists ("Creel", `creel.icns`), packaging (`Creel.app`;
    the dev script builds `Creel Dev.app`), user docs and the agent instructions' prose. The guard
    test `crates/ui/tests/brand_name.rs` fails if a user-facing "Zeron" comes back.
  - Logo: `dist/brand/` (generator, SVGs, PNGs) and `dist/macos/creel.icns`, with a simplified mark
    at 16 and 32 px. The in-app logo is `icons::logo(appearance)`: steel on light, moss on dark.
  - Left as Zeron on purpose: release file names (the update manifest matches on them), the
    stored-instruction preamble (hashed), and every identifier, environment variable, data path,
    bundle id, keychain entry, `zeron-artifact:` link and `zeron-agent` tool.
  - The wordmark is live Copperplate text; outlining it waits on a licence check.
- **Renamed to Mast** (2026-10-26; Mikey disliked Creel because it sounds like "creole"):
  - Name: about 330 more strings, both plists ("Mast", `mast.icns`), packaging (`Mast.app`, dev
    script `Mast Dev.app`), docs and READMEs. `brand_name.rs` now fails on a user-facing "Creel" too.
    The same identifiers as above stay "zeron".
  - Mark: a masthead light on a pole with two shrouds, a crow's-nest ring and three agent nodes.
    The light is a thin-rim glass lens with a moss → clover → cobalt gradient; the app icon has a
    metal finish (silicon, magnesium, fir, moss) on a steel tile. Only Cofactr brand colours.
    Generator: `dist/brand/generate.py` then `build-assets.sh`; the in-app marks are
    `crates/ui/assets/icons/mast-logo-{light,dark}.svg`, rendered in colour through `img()`.
  - `dist/brand/status/` holds the five lamp colours (idle, working, ready, input, error). Exploration pages and generators are in the
    Zeron Dev item's artifacts (`sources/mast-final/`).
- **Status light** (2026-10-26; `crates/ui/src/status_light.rs`):
  - The masthead light shows the most urgent state across chats: error (cadmium) > needs you
    (bromine; an awaiting-input chat or an open Required Action other than "Ready to merge") >
    ready (cobalt; an open "Ready to merge") > working (clover) > idle (clear glass). The pure
    `app_light` picks it; `Shell::sync_status_light` runs on state changes and the 1 s ticker.
  - Dock: icon swapped via `applicationIconImage` (embedded 512 px PNGs in
    `crates/ui/assets/dock/`) and a badge counting awaiting + errored chats + open Required
    Actions; debounced about 1 s, only changed parts touch AppKit. In-app logos use
    `Shell::logo`, with ten per-state marks generated by `dist/brand/generate.py`.
  - Settings → Notifications → "Live app icon" (default on); off restores the bundle icon, clears
    the badge and shows the plain logo. No pulse while working (a repeating animation redraws
    every frame).
- **PR chats start through the Start Chat dialog** (2026-10-27): every "start a review/fix-CI
  chat" entry point (outline ⌘↩ / Start Chat, context menu, Details, GitHub pane, PR page, URL box,
  palette) calls `Shell::open_start_pr_review`, which opens the Start Chat dialog in PR mode (PR,
  prompt opening, link, root, model/reasoning/approval pickers). Start sends `StartPrReview` with
  the picked config and `fresh: true` (new optional param; without it the engine reuses a live
  review chat and ignores the pick). "Open review" stays a direct open.
- **Every chat start passes an explicit model; the default is Opus 5.5** (2026-10-27): the
  built-in Claude Code default is `zeron_proto::DEFAULT_CLAUDE_MODEL` = `claude-opus-5-5` (added to
  the curated catalog; the `opus` alias labels as Opus 5.5; the picker prefers it over the live
  catalog's first row, which is the CLI default). MCP `create_chat` without `model`, work-item
  Start Chat/continue with no model, the PR review default config and `Pickers::resolved` without a
  catalog all fill it in, and the claude command builder always emits `--model` (empty → default,
  debug log). Most likely cause of the 2026-10-26 Fable chats: agents calling `create_chat`
  without `model`. Saved per-chat/remembered picks are untouched. Codex has no built-in default.
  Mikey switched the CLI default in `~/.claude/settings.json` to Opus 5.5 the same day.
- **Chats show the model they actually ran on** (2026-10-27): new `Chat.actual_model`
  (serde-default, no migration), stamped from Claude Code's init event (`SessionStarted.model`); a
  boot-time pass (`SessionsEngine::backfill_actual_models`) fills it read-only from each chat's run
  journal. `Chat::model_display()` (proto) drives the header, card/outline badge and tooltip
  ("configured X" when different, "default" when unknown). The composer chip on an existing chat
  reads "Next: <model>" when that is only the next turn's pick. MCP `get_chat`/`list_chats`: `model`
  is the actual model (else configured, else null) plus `modelIsActual`, `actualModel`,
  `configuredModel`. Claude Code only; Codex/ACP/OpenCode just echo the request, Cursor untouched.
- **Outline row author/assignee slots hug their text** (2026-10-27): `row_icons::text_slot` sizes
  the PR author / Linear assignee slot to its text (max 58px) and omits it when empty, so synced
  Linear rows no longer show a ~40px dead gap before "you". Icon slots are unchanged. (That column
  held the Linear identifier/state chip until the compact-icon change `528856b1`.)
- **Ticket ID in Linear rows' ticket link** (2026-10-27): the Linear status element (the one hit
  target that opens the ticket) shows the identifier (e.g. `ENG-3190`, 10px mono, `text_faint`)
  before the state dot and icon; one hover, tooltip and click. Hugs its content; empty identifier
  adds no width; nested rows too. Narrow rows drop the dot, then the assignee, then the ID.
- **PR page and Linear lists cache and revalidate** (2026-10-27): `engine/src/swr.rs` `SwrStore`
  (last good value per key, wall-clock freshness, one shared in-flight fetch per key, force
  bypass, optional persisted JSON with atomic writes off the async worker). The PR page persists to
  `<data_dir>/cache/pr-dash.json`; `PrDashboard` has `cachedOnly` (stored rows, never fetches), so a
  tab with no rows shows the stored copy at once, then fetches. Linear my-issues/my-projects/
  project-issues lists use the same store, and the Linear panel no longer resets to Loading on
  every open. A spinner shows only when nothing is cached; explicit refresh bypasses freshness.
- **Optimised install build** (2026-10-27): `[profile.installed]` in Cargo.toml (inherits release,
  incremental, codegen-units 256, no LTO/strip, line-tables-only debug). Clean build ~170s,
  rebuild after one UI file ~8s, binary 150 MB (debug was 286 MB), `target/installed` ~8 GB. Before
  this the installed app was a debug build with workspace crates at opt-level 0, likely the main
  cause of sluggishness. `debug_assert!` and overflow checks are off in it. Also: Overview `rows()`
  returns `Rc<Vec<OverviewRow>>` so hover/scroll frames no longer deep-clone every row.
  Outline rebuilds (2026-10-27, `perf-outline-rebuilds`): `AppState::overview_fingerprint` gates the
  Overview's state observer (rebuild + notify only when a row-relevant field changes);
  `WorkItemIndex` rebuilt only when a snapshot Arc is replaced; per-chat `row_fetch_due` stops
  `ensure_row_fetches` re-checking settled chats. 250 chats × 50 streaming updates: rebuilds and
  repaints 50 → 1, fetch checks 12500 → 0 (`OverviewPerf` counters). Still open: the other 15
  `AppState` observers (shell, transcript, composer, changes, terminal, files, history, pickers need
  broad state; chat_type_view, repository_topology, context_usage, artifacts tab and settings pages
  not audited; `chat_type_view` clones all chats every render).
  Model discovery (2026-10-27, `perf-model-discovery-cache`): harness `Catalog` success TTL is 30
  min (`CATALOG_TTL`, keyed on binary path/version/mtime/credentials); claude `InitializeCache`
  reused 30 min, a forced refresh reuses one from the last 120s, then re-probes.
  `backfill_actual_models` writes a `<chat>.modelchecked` sidecar (journal byte length) when no
  model is found and skips the chat until the journal grows.
- **Artifact zoom** (2026-10-27): `artifacts/zoom.rs` (25–800%, 10% key steps, continuous pinch
  and ⌘/Ctrl-scroll anchored at the pointer, per-artifact session memory, `ZoomSurface`). Keys ⌘= ⌘+
  ⌘− ⌘0 in the `ArtifactTab` context (registered from `find_view::bind_keys`; ⌘0 shadows "jump to
  session 10" only while an artifact has focus; keys propagate on Options/Source/Compare). Toolbar
  "− 100% +" (dim at 100%, click % to reset). Markdown: native rem-scaled text via gpui `PinchEvent`
  (reflows, vertical pan only). HTML/Mermaid: CSS zoom in the WKWebView frame script, page handles
  pinch (WebKit gesture events) and ⌘-wheel; unverified in a real WKWebView. Images: fit × zoom in a
  two-axis scroll. Not done: compare-pane sync, image drag-pan, Markdown px paddings.
- **Sandbox is enforced** (2026-10-28): Claude Code `read-only` passes `--disallowedTools
  Edit,Write,MultiEdit,NotebookEdit,Bash,PowerShell` + `--permission-mode default` and never
  bypasses permissions, even with auto-approve (so read-only Claude chats can't run git or tests).
  Claude `workspace-write` is NOT enforced (the CLI can't confine writes to cwd; unchanged).
  Codex `read-only` stays read-only with auto-approve forced off (it used to widen to full access).
  ACP harnesses (opencode, cursor, generic) refuse a `read-only` start with `HarnessError::Sandbox`.
  "No wider than the parent" holds in MCP `create_chat` and the work-item launch.
- **AppState observers gated** (2026-10-28, `perf-remaining-observers`): shell, composer, pickers,
  chat_type_view (rows cached in an Rc), repository_topology, context_usage, artifacts tab and the
  settings pages only notify when the slice they render changes (fingerprints like
  `overview_fingerprint`; counters in `shell/perf_tests.rs`). 250 chats × 50 updates: composer and
  pickers 50 → 0, shell 50 → 0–1; transcript still follows selected-chat streaming.
- **Artifacts phase 3** (2026-10-28): D: plan questions become Required Actions on the owning item
  (`engine/src/artifact_questions.rs`, reconcile every 2s on the owning device). Blocking = own action
  (key `artq:<artifactId>:<Q-n>`); the rest roll up into "N open questions in <plan>"
  (`artq:<artifactId>:rollup`; a changed list resolves the old one as "Superseded"). Answers live on
  the artifact row (`ArtifactView.answers`, `set_artifact_answers`); the plan file is never edited.
  Card answer → `ArtifactAnswerQuestion` RPC (messages the publishing chat, resolves the action);
  resolving the action marks the card answered. `artifact_get_plan` returns `answeredQuestions`.
  C: "Updated since you last viewed · rev N → M" chip opens the compare view; drift badge (commits on
  the item's chats' branches since the latest revision, or unreflected answers). Not done: answering
  from HTML cards, marker on artifact lists, multi-device answers are last-writer-wins. See the
  phase-3 amendment in `docs/internal/plans/artifacts-contract.md`.
- **Compare and zoom gaps** (2026-10-28): Markdown compare panes sync on scrollbar drag, paging and
  `scroll_to` (each pane reports position on paint; follower moves aren't echoed). Compare view is
  zoomable and panes zoom together (HTML/Mermaid frames mirror via SYNC scripts and
  `__zeronFrameCmd`). Zoomed images click-drag pan with a grab cursor and edge clamping. Markdown
  block gaps, list/quote/callout/question-card and preview padding scale with zoom.
- **Artifacts phase 5** (2026-10-28): F: "Side by side" on options artifacts shows 2–4 options in
  equal columns of one web doc (scroll + zoom synced across frames; >4 options pick which 4 via
  chips). Per-column "Pick this" / "Change pick to this" / "Picked · withdraw" with an optional
  note: stored as `ArtifactView.picked` (`ArtifactPickOption` RPC, files never edited), messages
  the publishing chat, resolves related Required Actions (`artopt:<artifactId>`, or the matching
  plan question), and `artifact_get_plan` returns `pickedOptions`. H: an item with a linked Linear
  project can mark one plan canonical; it shows beside the project's details (from the SWR-cached
  Linear data). Nothing is written to Linear.
- **Artifacts phase 4** (2026-10-28): frame → host bridge (frame postMessage → outer doc hidden
  `zeron-msg:JSON` link → WKWebView delegate → `BrowserEvent::DocumentMessage` →
  `ArtifactTab::on_frame_message`, validated in `artifacts/bridge.rs`; user-activated only, 16 KiB
  cap). HTML question cards are answerable in place (phase 3 path). Comments on HTML + Mermaid
  stored on the row (`ArtifactView.comments`, ≤40); anchors id/`data-zeron-anchor` → mermaid node
  → text-quote → CSS path; unfound = Outdated (`proto::resolve_anchor`). UI: "Comment on
  selection", "Pick element", "Comments N" panel, page markers. Unverified in a real WKWebView.
- **Compact single-chat rows** (2026-10-28): an item with exactly one live chat is one outline row
  (`ItemRow.compact_chat`, `OutlineOptions.compact_single_chat`); the chat's key aliases to the item
  row. Splits into item + chat rows on a second chat, a child item, a nested sub-chat, an archived
  chat (with show-archived) or a dangling chat.
  ⌘I opens Work Item Details for the selected row (compact or not); on compact rows the status
  glyph is clickable and opens details too (2026-10-28, `compact-row-details`).
- **Repository map removed** (2026-10-28): the UI (`repository_topology.rs`, route, button, ⇧⌘G),
  proto types and the `GetRepositoryTopology` RPC are gone; the shared engine discovery code lives
  on as `repository_discovery.rs`.
- **Linked Linear item** (2026-10-28): per-Space setting `linearAssignedWorkItemId`
  (`setLinearAssignedDestination`, needs Linear on). `engine/src/linear/assigned.rs` runs inside
  the Linear sync pass (15 min loop, manual Refresh, and when the destination is set) with the
  read-only query `ZeronLinearAssignedUnstarted` (viewer.assignedIssues, state backlog/unstarted/
  triage, project null) through the SWR store. Children "ENG-123 · title" are created via
  `createWorkItemFromLinear` with derived ids; tickets already linked elsewhere aren't duplicated.
  Tickets that stop matching: touched children get a marker, untouched ones are archived. System
  actor only, never on the undo stack; nothing is written to Linear.
- **Outline undo** (2026-10-28): `work_outline/action_log.rs` (in-memory per Space, cap 200, UI call
  sites only, so agent/remote/sync changes are never recorded) and `outline_view/undo.rs`. ⌘Z / ⇧⌘Z
  (and Ctrl-Z / Ctrl-Shift-Z) in the outline nav context; text fields keep their own ⌘Z. Steps go
  through the normal engine ops; a probe refuses when the target changed ("Can't undo … Nothing was
  changed"). Undo / Redo / Recent Changes… in the item and Space-root menus. Not persisted.
- **Small gaps** (2026-10-28): Markdown table/code padding, header and radius scale with zoom.
  Plan-question reconcile is event-driven (boot; outline kinds change, debounced 50 ms and gated by
  the kinds' generation marker; immediate after an answer/pick) with a 5-min safety sweep.
- **Web view ↔ composer focus** (2026-10-28, `browser/macos.rs`, `browser/mod.rs`): AppKit used to
  leave the WKWebView first responder after a click into the page or a forwarded shortcut, so keys
  (e.g. space) went to the page while GPUI showed the composer focused; page loads also stole GPUI
  focus. Now: a mouse-down outside the page restores GPUI's view as first responder (NSEvent local
  monitor, popover-aware); GPUI focus leaving the browser surface clears native focus; bare Esc in a
  page text field returns focus to the app; loads only take focus when it's already in the browser
  or nowhere (`may_take_focus`); plain keys are never forwarded. Still forwarded from page fields:
  ⌥↑/⌥↓ and Esc with the find bar open. HTML question-card typing: no blocker found, verify by hand.
- **Resizable panes** (2026-10-28): the chat-view sidebar (already draggable, `settings.sidebar_width`)
  now ranges 200–520 px and its shown width is clamped so the conversation keeps `CHAT_PANEL_MIN`.
  The Work Item Details / Required Actions panel has a left-edge drag handle (double-click resets to
  380), limits 320 px–60% of the Overview with the body ≥360, persisted as `detailsPanelWidth:N` in
  the Overview UI-flags file; drag only notifies the Overview.
- **Per-project agent accounts** (2026-10-28; plan zeron-artifact:05d0f52c4d8ddaae45242f8a3aaed976 rev 3,
  scope per project only): managed homes under `<data>/managed-accounts/<harness>/<id>/` (0700;
  device-local `store.json`). A project has a default account per harness; a chat resolves it at
  its first run and stays pinned (chats with an existing harness session pin to Default). Spawns,
  resume and title runs set `CLAUDE_CONFIG_DIR` / `CODEX_HOME` for the pinned home; Default sets
  neither and never touches ~/.claude / ~/.codex. Claude Code 2.1.285 keys Keychain as
  `Claude Code-credentials-<sha256(CLAUDE_CONFIG_DIR)[..8]>`, so homes are isolated (fallback switch
  `account_home::CLAUDE_KEYCHAIN_ISOLATED`). Sign-in runs `claude auth login` / `codex login` in the
  home. Skills/rules shared into homes (no credentials). UI: project row menu picker with Add
  account, chat account badge, Settings → Project accounts. Not done: usage rings and model picker
  per managed account, context-usage reader for managed homes, MCP/plugin sharing, Start Chat
  account chip, promoting old slots, API-key accounts. Needs Mikey's hand check: real sign-in and
  two accounts running at once.
  Sign-in and saved accounts (2026-10-29, `accounts-signin-and-saved`): `claude auth login` /
  `codex login` run on a PTY; the Add account sheet shows the URL (Open/Copy), any device code, and a
  "Paste the code" field (`SubmitManagedAccountLoginCode`; code goes to stdin only, never stored or
  logged), progress and Cancel. Saved Settings logins appear in the project picker and are promoted
  (`PromoteSavedAccount`) into a new managed home from Mast's own slot file: the live Default login is
  refused (use Add account); a promoted slot is moved (leaves the global switcher, Mast stops
  refreshing it) so two homes never share a rotating refresh token.
  Round 2 (2026-10-29, `accounts-round-2`): usage windows per managed account (same usage endpoints
  with the home's own token; never refreshes; 60 s shared cache) shown in Settings → Project
  accounts; context usage reads the pinned home (`CLAUDE_CONFIG_DIR/projects`, `CODEX_HOME/sessions`);
  Start Chat account chip with a one-chat override (hidden when the device has no managed account).
  Web-view ⌥↑/⌥↓ and Esc-with-find-bar no longer forwarded from page text fields. Open decision for
  Mikey: MCP server sharing needs reading the MCP section of `~/.claude.json` (holds identity data,
  never read today); plugins would need a copy-based share (symlink writes through to ~/.claude).
- **Internal rename zeron → mast** (2026-10-28, `rename-zeron-to-mast`): crates are `mast-*` (`mast-ui`,
  `mast-engine`, `mast-harness`, `mast-mcp`, `mast-proto`, …), the app package is `mast` (`apps/mast`)
  but the binary is still `zeron`, so the install command is `cargo build --profile installed -p mast`
  and the output stays `target/installed/zeron`. Runtime names write the new form and accept the old:
  `MAST_*` env (`ZERON_*` still read via `mast_proto::env`), MCP server `mast-agent` (`zeron-agent`
  alias), `mast-artifact:` / `mast-file:` / `mast-msg:` / `mast-link:` schemes, `data-mast-*`,
  `.mast/` artifacts folders (existing `.zeron/` folders still used), `mast.json` (`zeron.json`
  fallback), `mast/<name>` worktree branches (`zeron/` still recognised). Not renamed (see
  `docs/internal/plans/rename-migration.md`): `~/.zeron`, bundle id `sh.zeron.app.dev`, Keychain names, the
  `zeron` binary, the `zeron/` submodule folder. `scripts/rename-map.sh` maps upstream code onto the
  new names. New git spawns use `mast_harness::git_command()`.
- **Chat header revamp** (2026-10-28, option C + B's card on hover): `chat_item.rs` fact sheet feeds
  three surfaces. Header: clickable trimmed breadcrumb, title, chips (model actual/configured,
  account, branch, PR, ticket), ⚠ / ● badges opening the Item tab. Sticky "Item" tab (first in the
  right pane, not closable): path, instructions (Edit, Compare/Apply, proposals), Runs in, Model,
  Account, Links, Needs you, Plan, Children; no-item chats get Runs in, Model, Account, "Add to
  outline". Hover card: ~400 ms show, 160 ms grace, ⌥⌘I pins; never takes focus. ⌥⌘; opens the
  Item tab. Removed: the work-context strip, drift and proposal lines, and the header's workspace /
  chat-id / session-id chips. Follow-up (`header-followups`, 2026-10-29): an "Identifiers" section at
  the bottom of the Item tab (chat id, session id, workspace, each with Copy); `open_web_reference`
  tests rewritten in `chat_item_tests.rs::web_references`; visual polish to the mockup (semibold title
  under the breadcrumb, icons and tones on chips, tighter Item tab sections, condensed hover card).
- **Perf baseline** (2026-10-29, `perf-baseline`; report mast-artifact:65d48729c91ee81b655f90b6adda143b,
  commands in `docs/perf-baseline.md`): `#[ignore]` benches for transcript/markdown/diff/overview
  render, engine startup, SWR cache, journal scans and git spawn (run with `--profile installed`; some
  use a read-only copy of the real profile with a throwaway HOME). Fixes: transcript rail ticks cached
  (frame 7.8 → 0.5 ms, streaming tick 17.7 → 3.2 ms at 2000 messages); registry rows read in place and
  only changed kinds published (heartbeat 2.14 → 0.13 ms); journal backwards tail scans
  (`stale_sessions` 440–635 → 5–7 ms); boot recovery reads journal tails (`EngineCore::assemble`
  ~940 → ~350 ms). Running app under load: ~61% of a core mean (engine, not UI thread), RSS swings
  1.6–6 GB. Open findings (ranked): Overview List mode not virtualized (44–55 ms/frame, 335 MB alloc at
  250 chats); Work Outline recompute on every chat/session write (33–36 ms); diff-sync runs 7 git
  processes per kick, every 120 s for 22 repos; transcript sync O(entries) per tick; artifact question
  sync re-sent an op id with a changed payload (fixed in round 2); RSS swings unexplained.
  Round 2 (2026-10-29, `perf-round-2`): question-sync op ids derive from the full payload (origin, text,
  info, key), fixing the `operationConflict` writes; Work Outline recompute gated by a watch marker
  (outline-kind generation + per-Space chat/session fingerprint): ~46 ms per heartbeat → 0.7 ms check
  (`bench_work_outline_recompute_per_heartbeat`); diff-sync full capture 7 → 5 git processes, repair
  tick runs one `git status --porcelain=v2 --branch` probe and skips unchanged checkouts, archived-only
  checkouts aren't kicked. Still open: Overview List mode virtualization, transcript sync O(entries).
- **Changed-since marker on artifact lists** (2026-10-29): accent dot + "Updated since you last viewed:
  rev N → M" tooltip on Details panel artifact rows, the Item tab Plan row and the outline row's
  artifact chip, from `settings.artifact_last_viewed` (`questions_ui::list_marker`).
- **Docs for coworkers** (2026-10-29, `docs-onboarding`, docs only): README rewritten (what/why, install,
  first run, keys, docs links, credits), README.zh-CN.md removed, guides in `docs/guide/`
  (getting-started, work-outline, orchestrating, plans-and-artifacts, github-and-linear, accounts,
  keyboard) with "Try it" walkthroughs. Image/recording TODOs for Mikey are listed in the guides. The
  repo-cleanup proposal (Part B) awaits Mikey's approval: mast-artifact:bacfa5c96fe27d73a2821e8f4e2aa446.
- **Side by side fits designs** (2026-10-29, `side-by-side-fit`): each option frame lays out at its
  design width (`designWidth` on the manifest entry, `data-mast-design-width`, manifest-wide, default
  1100, clamped 320–4000) and is scaled to the column with a CSS transform (never above 1x). Toolbar:
  2/3/4 up, per-column Widen/Narrow (2.2x share), Fit / 100% (real size, horizontal scroll); remembered
  per artifact (`artifactSideLayouts`). `crates/ui/src/artifacts/side_layout.rs`; conventions doc §6
  and the artifact_publish description tell design agents to set the width.
- **Item tab A1 + Team tab** (2026-10-29, `item-tab-team`; design mast-artifact:b30aa528f035e921c286d66a56eb7606):
  Needs-you block at the top of the Item tab (Required Actions for the item + descendants) grouped
  Blocking → Ready to merge → FYI, oldest first, 2-line clamp with Details, inline Reply & resolve /
  Resolve / Open / Approve; same grouping in the Details panel and the global Required Actions panel.
  Other Item tab sections are one-line accordion rows. Team tab (`team.rs`, `shell/team_ui.rs`) appears
  only for chats with workers (child chats or chats on descendant items): per-worker state, last
  activity, PR/ticket chips with status, diff stats and Ready-to-merge jump, with a roll-up line; the
  Item tab shows the roll-up and top workers.
  PR rows in Item tab Links and the hover card (2026-10-30, `item-tab-pr-status`): state-coloured icon,
  truncated title (tooltip with repo#n, state, checks, review), Open/Draft/Merged/Closed label, checks dot,
  review decision, from the GithubStore cache (uncached → "PR #N" until `request_status` lands). Tickets
  show "ID title" plus Linear state when known. Team tab PR chips are a separate component (unchanged).
- **Project templates, increment 1** (2026-10-29, `project-templates`; design option A
  mast-artifact:d32bb551266cd62ceef33f16477b6c1a): Group into… (⌘/⇧-click multi-select item rows,
  right-click or ⌘⇧G; new container or existing item; one undo step). Per-item `isProject` and
  `workerDestinationId` (Details → Project section; users only). `work_outline_create_child` with no
  parent resolves to the nearest project's destination; `create_chat { workItemId = project }` starts
  in it; project scope (contract amendment in m0-contract.md) lets a chat on a project's Planning item
  create into its sibling Tickets container but nothing outside the project. Gap: a bare `create_chat`
  without `workItemId` still inherits the caller's item (fixed in increment 2).
  Increment 2 (2026-10-30, `project-templates-2`): templates copy ANY item subtree (titles,
  descriptions, instructions on every node, project flag, worker destination; not chats, Required
  Actions, external links, archived items or tickets under a destination). Save as template… on any
  item (Update template = rev+1, or Save as new); Space-root details → Templates (list, tree preview
  with per-node instructions, New from template…, Delete). New from template… creates exactly that
  tree under a chosen parent with a chosen root title (item menu, Space-root menu, Templates list).
  Template-changed review for copies; Convert to project maps an existing item onto a chosen
  template. A bare `create_chat` (no `workItemId`) from a chat inside a project with a worker
  destination creates a new item there and starts the worker on it.
- **Running-chats popover** (2026-10-29, `running-chats-popover`; design A revised
  mast-artifact:f5bdd9f1c79754065ba039d4e8b0d62f): titlebar lamp button (next to +) or ⌥⌘R opens a
  popover: Needs you first, then by Space, children under coordinators; rows show title, last message
  preview, elapsed, item path, account, model. Keys ↑↓, ↩ open, ⌘↩ open in Overview side pane, I
  interrupt, →/← fold, Esc. Pending build: the launch executable's inode/mtime/length is re-checked
  against the installed bundle every 5 s; a newer one rings the lamp and shows "New build installed ·
  N changes" (sha embedded by `apps/mast/build.rs`, so the list starts from the next install) with
  Restart when idle / Restart now… / Restart. No restart UI without a newer build. Interrupted ledger
  in the engine (list/dismiss/resume RPCs): "Restarted HH:MM · N interrupted" with Resume, Dismiss,
  Resume all. "Idle + background process" is rendered but the engine doesn't track background
  processes yet.
  Titlebar layout fix (`fix-titlebar-lamp-overlap`): `TITLEBAR_TRAILING_SLOTS_WIDTH` always reserves the
  + and lamp slots, so titles never start under the lamp and the lamp's position is stable on every route.
- **Instruction proposals edit/remove** (2026-10-30, `instruction-proposal-edits`): proposals carry
  `action` add | edit | remove, `instructionId` and an optional `rationale`; edit/remove record the base
  text + updatedAt. Accepting an edit rewrites the same instruction (history/drift intact); a remove
  archives it; stale proposals are refused unless Apply anyway (`allowStale`). Agents may only target
  instructions in their item's effective set. Details → Proposed instructions shows a word diff
  (`proposal_diff.rs`), struck-through removes, the rationale, Accept / Edit then accept / Dismiss.
- **Plan question typing** (2026-10-30, `fix-plan-question-typing`): the Markdown preview root focused
  itself on mouse-down after the answer input did, stealing focus; the input wrapper now stops
  mouse-down (`markdown/render.rs`). HTML question cards: no blocker found; debug logging on target
  `focus` (web view mouse-down in_page/hit view, key routing, focus hand-backs) for a hand repro.
- **Menu bar item** (2026-10-30, `menu-bar-item`; design A + C's mini bars,
  mast-artifact:4b4633204f14e4dc1e6d8921620ac805): `crates/ui/src/menu_bar.rs` (pure menu model:
  build_menu, usage_accounts, format_reset, lamp_rgba, near-limit rule; thin objc glue: NSStatusItem +
  NSMenu, `MastMenuBarTarget`) and `shell/menu_bar_ui.rs`. Glyph: lamp in the state colour (idle
  silicon, working clover, ready cobalt, needs-you bromine, error cadmium), fir ring for a newer build,
  plus one mini bar per account (≤4, height = 5h %, bromine ≥80%, cadmium ≥95%). Menu: per-account
  usage cards with reset times, chats (Needs you first), interrupted + Resume all, Restart when idle
  (newer build only), Open Mast, Quit. Uses the existing usage cache; AppKit glue is compile-checked
  only (hand-verify). Also supersedes the retired peon tray.
  Usage fetch fix (2026-11-02, `menu-bar-usage-fetch`): `UsageSchedule` probes at startup (once the
  engine answers), on menu open (if ≥60 s old) and every 5 min; failures retry after 60 s. Rows are
  enabled (AppKit dims disabled items) with label/secondaryLabel colours and solid bars. Cards say why
  usage is missing (token expired, not signed in, sign-in not readable, API key); debug log target `usage`.
- **Orchestrator wakes** (2026-11-02, `orchestrator-wakes`): engine `orchestrator_wakes.rs`, event-driven
  (outline snapshot Required Actions raised/resolved, session watch errored/turn finished, PR linked).
  Workers = child chats + chats under the project worker destination. Events batch over
  `WAKE_WINDOW_MS` (45 s) into one queued system "Worker updates:" message, delivered only when the
  orchestrator is idle; held while Mikey has a draft, was active in the last 2 min, or has a queued
  message; a user send during a wake turn interrupts it and runs next. Defaults: on, Required Actions +
  errors; turn-finished and PR events off. Settings in `<data_dir>/orchestrator-wakes.json`. UI: Team
  tab header (toggles, "N updates held" + Deliver now, unseen marks), Item tab Wakes row. Gaps: PR
  merged not fed yet, held events in memory only, only the orchestrator's host device delivers.
- **Release 2026-11-02 evening** (installed 18:20; tests confirmed later): `worker-report-to-parent`
  (a chat may `send_message` its parent or call `report_to_parent`; reports go through the wake queue as
  a "worker report" event, ≤4 KB, on by default, Team tab activity line), `wait-cap-and-link-inference`
  (`wait_for_turn` / `send_message {wait}` stop at 1500 s with `stillRunning`; replies capped with
  `truncated` + paging; stricter PR/ticket "mentioned" inference), `relay-user-answer`
  (`answer_user_action_request { requestId, userMessageId }` delivers the user's own message as a
  user-attributed Reply & resolve after checks: role=user, human-authored, newer than the request, in
  reach; project-scope `read_chat`/resolve via `Reach::ReadScope`), and `container-templates` phase 1
  (template rules in a container's Details; New work item under the container materializes the
  structure with live instructions, add + mute; moved-in items marked "doesn't follow template" with
  Apply; `work_outline_apply_container_template`; plan mast-artifact:df5be7ef1a2fa657e3737f17dd0ef89a lists
  the remaining phases). Merge conflicts in crates/mcp/src/tools.rs resolved by hand keeping both sides.
- **Admin chats** (2026-11-02, `admin-chats`, installed 21:10): right-click a chat (outline/sidebar) or
  the Item tab → "Grant admin for this Space…" (1 / 4 (default) / 8 / 24 h or until revoked; optional
  "Skip bulk confirmations", off by default). Badge "Admin · <Space> · Nh left" + Revoke in the header
  and Item tab; shield tag in the running popover and menu bar. Grants are re-checked per call and per
  batch step and only cover the granted Space: list/get/read/send/wait/archive any chat, create_chat,
  read_subtree and answer_user_action_request on any item, and `work_outline_admin` batches (incl.
  container-template set/apply/rule edits). Attribution "admin chat … (granted by Mikey)"; sent
  messages say they are not Mikey's own. Bulk batches raise a preview Required Action unless skipped;
  per-turn undo from Recent Changes.
- **Template-aware instruction proposals** (2026-11-05, `template-instruction-proposals`, Terra): edit/remove proposals on
  a template-derived instruction (`templateInstructionId`) now target the container's rule instruction; approving
  goes through `setContainerTemplate` and propagates to every following child. New `childTemplate` add target (rule
  id or title path; defaults to the caller's container/rule node). Review card: "Child template · applies to N items"
  with Apply to template / This project only (adds an own instruction). Guard: approval can no longer write a
  template-linked row directly (it used to, and the next template save silently overwrote it). Release run: nextest
  2388/2388, UI 2489/0.
- **Rename for distribution** (2026-11-05, `rename-for-distribution`, Terra; decisions: bundle id
  computer.mikey.mast / .dev, `~/.mast` for new installs, keep `~/.zeron`): binary `mast` with a `zeron` shim bin;
  updater and `daemon install/status` accept both; `CFBundleExecutable mast`; both `mast://` and `zeron://` declared,
  `mast://` generated; one-time copy of the old `sh.zeron.app[.dev]` UserDefaults domain; data dir: `MAST_DATA_DIR`/
  `ZERON_DATA_DIR`, else `~/.zeron` if it exists and `~/.mast` doesn't, else `~/.mast`; Linear Keychain service
  `computer.mikey.mast.linear` with read fallback to `sh.zeron.linear` (copied forward, old kept);
  `scripts/install-macos-app.sh`; docs/guide upgrade note. Wire names unchanged. First restart re-asks macOS
  permissions and the Linear Keychain item. Release run (`--all`): nextest 2434/2434, UI 2488/0.
- **Distinct status indicators** (2026-11-05, `status-indicator-distinct`, Terra): one accent-independent state style
  (colour + glyph) shared by Team, running chats, sidebar, outline, item details, Overview, review chats, menu rows
  and Needs-you markers: Working clover/juniper spinner, Waiting bromine "!" badge, Errored cadmium ✕, Ready cobalt →,
  Done fir ✓, Idle silicon/steel ○. Previously Working used the user's accent and could match Waiting's amber. Light
  fir on the sidebar is ~2.95:1 (glyph + label carry it). Release run passed, UI 2488/0.
- **Start Chat sign-in preflight** (2026-11-05, `start-chat-signin-preflight`, Terra): read-only cached (≈30 s) local
  sign-in detection (the doctor detectors) → `signed_in | signed_out | unknown`; the new-chat composer and work-item
  Start Chat show a nonblocking "isn't signed in" notice with Sign in… (Default → Settings → Agents; managed → Space
  accounts, using the signed-out flag) and Start anyway; unknown shows nothing. Release run: nextest 1274/1274, UI
  2485/0.
- **Stale coordinator wakes** (2026-11-05, `stale-coordinator-wakes`, Terra): worker events go to one eligible
  coordinator: a live explicit parent wins; otherwise one non-archived, wake-enabled chat on the project item (latest
  hand-off target or most recently active), never every chat on the item. Archived and wake-disabled coordinators get
  nothing (auto-continue and durable restore included); sending to an archived coordinator unarchives it. Known
  follow-up: a cross-device archive→unarchive race can let one held wake through. Release run: nextest 1242/1242,
  UI 2481/0.
- **Talk/decision metrics** (2026-11-05, `talk-metrics`, Terra; plan rev 3 phase 0): `tools/mast-metrics` reads an
  immutable copy of the profile (sqlite `.backup` of docs.sqlite3 + copied journals, never the live store) and writes
  aggregate JSON/Markdown with `--since/--until` and `--compare <old.json>`. Snapshots and reports live in
  `~/mast-metrics/` (never committed); re-run steps in tools/mast-metrics/README.md. Baseline (14 days to
  2026-11-05): 406 cards vs 872 heuristic prose questions; card replies median 9.4 h / p90 136 h (n=33); coordinator
  replies median 171 words; 72 status checks. Summary mast-artifact:49d61202530e739e159d03b99bb0c7c8. Re-run ~2026-11-12.
- **`zeron doctor`** (2026-11-05, `mast-doctor`, Terra; merged after Mikey's go-ahead): read-only setup checklist
  (`zeron doctor`, `--json`): agent CLIs (login-shell PATH, exact override paths, rustup shims kept), local agent
  sign-in, gh + auth, git/LFS, data dir access/size/free space, engine lock + IPC, Rust toolchain, Linear Keychain
  presence. ✓/!/✗ with a fix per line; errors exit 1, warnings 0; missing both Claude and Codex is an error. README
  and getting-started mention it (conflict with the terminology pass resolved). Release run: nextest 2411/2411, UI
  2481/0. A shorter `mast` command belongs to the binary rename.
- **Perf sweep 5: startup main-thread ticks** (2026-11-05 overnight, `perf-sweep-5`, Terra; gated instrumentation
  only): per-view startup spans name WatchSpaces and AuthStatus updates as the first two long main-thread ticks; GPUI
  test-support draws dirty windows synchronously during effect flush (inflates headless numbers). A third tick
  remains unattributed. Startup perf work stopped here (diminishing returns). Release run passed (UI 2481/0).
- **Test home isolation guard** (2026-11-05 overnight, `test-home-isolation`, Terra + coordinator fix 0c9ed930): a
  test_support initializer gives every test process its own temp HOME, CLAUDE_CONFIG_DIR, CODEX_HOME (created; the
  codex CLI refuses a missing one) and Mast data dir, preserving inherited fixture homes for re-executed test
  binaries; debug/test builds refuse engine and local-profile data paths inside the original home; three live
  transcript smoke tests replaced with synthetic fixtures; `linear_assigned` waits for archive + reason + sync flag
  together (settle race). `scripts/check_test_isolation.sh` (+ allowlist) fails on new home-dependent references in
  test code. Audit: docs/internal/audits/test-home-isolation.md. Release run: nextest 2411/2411, UI 2481/0 (first run
  with the guard caught the CODEX_HOME regression; fixed).
- **Needs-you phase 6** (2026-11-05 overnight, `needs-you-phase-6`, Terra): notification banners and the menu bar's
  needs-you section show the request kind, one-line ask, origin chat/item and age, ordered by priority then age;
  quick actions (choose options capped at 3, Approve/Decline) go through the card relay/resolve path (idempotent,
  banner/menu provenance). `fyi` doesn't banner by default; existing banner settings respected. Release run:
  nextest passed, UI 2481/0.
- **local_import flake fix** (2026-11-05 overnight, `flaky-local-import`, Terra; test code only): each local_import
  test runs in its own process with a private HOME and data dir. Root cause: background external-session adoption
  read the real HOME's sessions and raced the fixtures. Stress 1/20 → 20/20. Release run: nextest 1223/1224; the 1 =
  `linear_assigned::an_untouched_child_is_archived_when_its_ticket_leaves` (assertion at tests/linear_assigned.rs:504,
  "Canceled in Linear" not yet set: a settle race under load; the binary passed 9/9 three times alone). **New flaky:
  linear_assigned.** UI 2475/0. A test HOME isolation guard is in progress (`test-home-isolation`).
- **Perf sweep 4: startup gaps** (2026-11-05 overnight, `perf-sweep-4`, Terra; gated instrumentation only): chats are
  applied ~4.5 ms after core assembly; outline wire encoding 0.11 ms for a 179 KB frame; the 37–52 ms watch
  spawn-to-first-poll interval sits inside one headless GPUI test-executor drain (a 25–31 ms tick plus ~11 ms
  ticks), so it's a benchmark artifact as measured. Registry work finishes before watch spawn; PR fetch/preview
  scanning already use spawn_blocking. Open lead: identify the 25–31 ms main-thread runnable at startup (likely first
  layout). Release run: UI 2475/0; nextest had 2 `local_import` failures under load; the binary passed 8/8 twice
  alone. **New flaky: the `local_import` integration binary** (3 different tests failed under load tonight;
  assertion at tests/local_import.rs:439), so it needs a reliability pass.
- **Cleanup sweep 4** (2026-11-05 overnight, `cleanup-sweep-4`, Terra; mast-ui only, no behavior change): declared the
  objc `cargo-clippy` cfg in crates/ui/Cargo.toml (removed ~141 macro warnings), removed unused code/imports,
  fixed visibility, mechanical clippy. mast-ui lib-test warnings ~169 → 15. Release run: nextest passed, UI 2475/0.
- **Perf sweep 3: startup phases** (2026-11-05 overnight, `perf-sweep-3`, Terra; measurement + instrumentation):
  sidebar draw is ~10 ms (not a problem); hidden sidebar jump ordering skipped. Startup medians of 5 serial runs on a
  53 MB profile: core assembly 85 ms, registry read 12 / decode 21 / settle 22 / save 9 ms, chats observed 242 ms,
  outline observed 317 ms (construction 1.3 ms), loaded 326 ms. Gaps under investigation in perf-sweep-4. Release
  run: nextest 1224/1224, UI 2475/0.
- **Cleanup sweep 3** (2026-11-05 overnight, `cleanup-sweep-3`, Terra): mechanical clippy fixes in mast-engine in
  six module-group commits (no MCP changes needed; mast-mcp had 0); engine diagnostics 431 → 382. No API, serde,
  protocol or behavior change. Release run: nextest 1224/1224, UI 2475/0.
- **Perf sweep 2: startup** (2026-11-05 overnight, `perf-sweep-2`, Terra): font catalog inspection deferred until
  needed (registration 185 ms → 0.01 ms; installed font selections still resolve before drawing, picker complete);
  cached chat revisits skip the unused provisional tail. Initial shell draw on a 53 MB profile 203 → 132 ms. Note:
  the "loaded sidebar 384 ms" figure is cumulative startup, not the sidebar draw (~10 ms; perf-sweep-3).
  Release run: nextest 2404/2405 (`local_import::marker_persistence_failure_is_an_import_error`, load flake, passed
  twice alone), UI 2475/0.
- **Cleanup sweep 2** (2026-11-05 overnight, `cleanup-sweep-2`, Terra): mechanical clippy fixes, no API/serde/
  protocol/behavior change. Clippy warnings: harness 14→1, doc 29→19, proto 18→10, sync 12→6 (with
  `mock-server`); rpc and update had none. Note: `mast-sync --all-targets` needs `--features mock-server` to build
  its integration tests. Merge conflict with phase 5 in user_actions.rs resolved (let-chain + `prMerged` rule).
  Release run: nextest 2355/2355, UI 2473/0.
- **Needs-you phase 5 subset: handoffs land** (2026-11-05 overnight, `needs-you-phase-5-land`, Terra): `kind: handoff`
  cards show a PR lifecycle chip from the PR cache (Ready / Waiting to land / Landed / Closed, including PRs linked
  from other chats); on a PR-merged event the matching open handoff resolves once ("Landed: repo#n merged …",
  actor "Mast (PR merged)"). Per-Space setting "Resolve handoffs when their PR merges", default on. Local
  `--no-ff` merges without a PR stay manual. No Merge button (not built). Release run: nextest 2355/2355, UI 2473/0.
- **respond_to_input validation** (2026-11-05 overnight, `respond-to-input-validation`, Terra): answers are checked
  against the pending request's question ids and option labels before queuing and again before resolving; a
  single unknown or omitted id maps to the only question; otherwise an error lists valid ids/options and nothing
  is sent. An unmatched answer can no longer become a Codex approval denial (friction 4a607d99). Release run:
  nextest 2354/2354, UI 2471/0.
- **Flaky test fixes** (2026-11-05 overnight, `flaky-tests`, Terra; test code only): Devin discovery startup
  deadline 500 ms → 30 s (test now ~94 s by design); device-relay fixture no longer writes the live
  `~/.zeron/worktrees` (per-engine temp worktree root); Codex override login fake keeps running via /bin/sleep
  (still quarantined). Each fixed test 20/20 under 16-way concurrent load. Release run: nextest 1682/1682 (first
  clean engine run tonight), UI 2471/0.
- **Cleanup sweep** (2026-11-05 overnight, `cleanup-sweep`, Terra; no behavior change): obsolete iOS/mobile and
  Windows docs corrected, demo script name, unused compiler constructs and imports removed, two clippy deny lints
  in updater test fixtures fixed. Not done: the remaining clippy backlog and ~169 UI test warnings (141 from the
  objc macro's `cargo-clippy` cfg). Settings fields with no reader kept for stored compatibility:
  `sidebar_grouped`, `right_pane_open`, `terminal_open`. Release run: nextest 1617/1617, UI 2471/0.
- **Needs-you phase 4** (2026-11-05 overnight, `needs-you-phase-4`, Terra): optional `options: [{id, label,
  detail?}]` on create_user_action_request (MCP, proto, registry; old requests unchanged). Cards everywhere offer
  choice buttons + "Other…", an inline reply for `ask` (⌘Enter), and 1–9 picks on a focused card; each relays via
  the phase-2 `Re: "…"` format and resolves once. Coordinator fix 2c514432: dropped `options` from the MCP server
  instructions (6 chars over the 1900 budget; the tool description documents it). Release run: nextest
  2352/2353 (that budget test, fixed), UI passed.
- **First-run follow-up** (2026-11-05 overnight, `first-run-followup`, Terra): PR dashboard no longer shows raw
  `gh` auth output or "No pull requests" after a failed fetch; common auth and missing-CLI errors get a clear
  next step; signed-out fake-Claude harness test added; findings appended to docs/internal/first-run-audit.md.
  Release run: nextest 1680/1682 (`e2e::wrong_id_respond_is_rejected_and_correct_answer_still_resumes` and
  `m5_repos_diffs_terminals::git_history_search_is_fuzzy_complete_and_accepts_sha_prefixes` failed under load,
  passed twice alone), UI 2468/0.
- **Rebindable outline and Team shortcuts** (2026-11-05 overnight, `rebindable-outline-shortcuts`, Terra): the
  outline's default action table (with aliases such as F2/R, ⇧→/Tab, ⌘Z/Control-Z), Shift-F10/Menu and both
  Team shortcuts (⌥⌘T global, `T` in the side panel) are persisted Settings → Shortcuts records under Outline and
  Team; rebinding replaces the action's defaults, updates hints, and conflicts are checked by physical key and
  context. Row-edit Enter/Escape and pickers stay hard-wired. Release run: UI 2467/0, nextest passed.
- **Perf sweep: List virtualization** (2026-11-05 overnight, `perf-sweep`, Terra): List mode virtualizes
  variable-height rows, keeping grouped order, expansion, selection, scroll targets and visible anchors across
  reorder. 250-chat redraw 105.66→1.98 ms mean (p95 150.6→2.5), first draw 68.7→2.5 ms. Measured but not
  changed: prepared 2,000-message transcript open+draw 9.7 ms (prep 74 ms off-thread), production streaming
  5.8 ms/chunk mean (unchanged rows reused), Team facts 0.07 ms, outline redraw 3.2 ms. Benchmarks and method
  in docs/internal/performance-sweep.md (3 ignored bench tests). Release run: nextest passed, UI 2461/0.
- **First-run audit fixes** (2026-11-05 overnight, `first-run-audit`, Terra): report at
  docs/internal/first-run-audit.md (6 findings). Fixed: profile classification (fresh vs existing) now uses
  saved settings or real Space/chat registry data before either startup mode, so headless-first startup no
  longer marks a fresh profile existing; packaging uses the binary Cargo reports (CARGO_TARGET_DIR-aware) and
  fails clearly when missing; launch returns "agent not installed: <harness>" up front instead of "started";
  docs no longer present gh as required; PR repository error lost its Cofactr wording. Open from the audit:
  signed-out-but-installed agent flow, sign-in gate and no-auth tab states not yet checked. Release run:
  nextest 2401/2401, UI 2459/0.
- **PR file paging** (2026-11-05 overnight, `pr-files-paging`, Terra): PR files fetched page by page up to 3,000;
  partial results kept on failures and rate limits; scores reused for an unchanged head sha; "partial" badge
  with "Scored from the first N of M files". Release run: nextest 2350/2350, UI passed.
- **Diff navigation perf + friction import** (2026-11-05 overnight, `perf-diff-navigation`, `friction-import`,
  Terra): navigation keeps the flattened rows when the target is already expanded; the file sidebar is
  virtualized (diff body already was). 300-file synthetic diff: first draw 30.9→1.7 ms, hunk key 52.5→3.4 ms,
  file key 48.6→2.7 ms, scroll 22.9→2.0 ms. Friction "Import…" parses the old agent-friction-log.md format and
  Mast's export (round-trips), preview, redaction, fingerprint dedupe, idempotent re-import, import provenance.
  Release run: nextest 2347/2348 (the 1 = `devin_discovery_errors_and_timeouts_retain_last_good_then_recover`,
  timing flake, passed twice alone; no harness changes), UI 2456/0.
- **Hand-off UI + wake labels + child file limit** (2026-11-05 overnight, `hand-off-ui`, `wake-label-fd-limit`,
  Terra): "Hand off to new chat…" in chat context menus and header overflow (only with live children): sheet
  with prefilled summary, worker list, optional archive of the source; selects the successor; the source shows
  a linked "Handed off to …" line. Wake/report lines resolve the registry chat title at delivery (fallback
  short id, never a model name). Harness child processes raise their RLIMIT_NOFILE soft limit to
  min(hard, 8192). Release run: nextest 2345/2345, UI 2454/0.
- **Space account re-sign-in, durable wakes, drop any file** (2026-11-05, `managed-account-reauth`,
  `durable-wakes`, `composer-drop-any-file`, Terra): Settings → Space accounts has **Sign in again** (same home
  and id, Claude and Codex); auth failures mark the pinned account signed out and the chat error offers
  "Sign in again…". Worker wakes/reports persist in a bounded store (≤500 per orchestrator, atomic
  temp+rename, writes only on record/queue/deliver/prune, off executor threads): UUID event ids, restore
  before any queue drain, recovered wake-only queue rows re-held (user rows stay frozen), `delivered` pruned
  after the snapshot generation persists (legacy 3 s fallback); a crash between dequeue and transcript write
  can duplicate at most once, never lose. Composer: Finder drops and pasted files of any type: images attach,
  workspace files/folders become mentions, outside files insert absolute paths, remote-device chats and
  unreadable paths show a notice. Merge conflict in engine lib.rs resolved (durable's early wake service
  construction + PR-merged wiring after `spawn_watchers`). Release run: nextest 2058/2058, UI 2449/0.
- **Terminology pass** (2026-11-05, `terminology-pass`, Terra; text only): copy templates are **Blueprints**
  ("New from blueprint…", "Blueprint changed"), container templates are **Child template** ("Set up child
  template…", "Child template applied"); glossary in docs/work-outline.md (Space = the folder/outline root,
  project = an outline item marked as a project). Identifiers, tool names, RPCs and stored keys unchanged.
  Release run: nextest 55/55 (selected), UI 2445/0.
- **Hand-off + quick wins** (2026-11-05, `hand-off-orchestrator`, `pr-merged-wake`, `diff-target-remove`,
  `visual-compare-banner`, Terra): `hand_off_orchestrator` MCP tool stages a successor chat on the same item
  (same harness/model/sandbox), reparents active workers before its first turn, moves held/queued wakes,
  copies wake settings and the auto-continue override, records provenance, idempotent retry; first message
  lists workers and open requests. PR-merged wake now fires on owned PR open→merged (deduped, persisted
  marker; names open handoff requests for that branch). Changes picker: Remove per bound target (not the
  primary), limit message points at it, missing target paths pruned. Visual compare: "compared the first N
  px" banner in side-by-side and onion; onion follows shared zoom. Release runs: nextest 2318/2318 then
  1196/1196, UI 2443/0 then 2445/0.
- **Auto-continue** (2026-11-04, `auto-continue`, Terra, 3 slices): when a worker ends a clean turn with its item
  still open (no open Required Action, not awaiting input, no refused tool calls, no user send during the turn,
  nothing queued), Mast sends it a `[Mast: auto-continue n/3]` nudge; after 3 it queues one "worker stopped
  after 3 auto-continues" orchestrator wake. Counter per worker chat per item, reset by a user or orchestrator
  message, persisted across restarts. Off by default (global `autoContinue` in `orchestrator-wakes.json` plus
  per-orchestrator overrides); toggle "Auto-continue workers" in the Team tab header and Item tab Wakes row.
  Known edge: an orchestrator saying "stop" without completing the item or raising a request still gets nudged.
  Release run: nextest 2306/2306, UI 2443/0.
- **Wake echoes + archivable PR rule** (2026-11-04, `wake-echo-redelivery`, `archivable-pr-rule`, Terra): a
  resolved-action wake is suppressed when the receiving orchestrator resolved the request itself (user and
  other-chat resolutions still wake); wake batch selection and queueing are serialized, so concurrent
  evaluations can't deliver the same batch twice. A chat is PR-finished only when every PR it owns is merged
  or closed; a PR with no cached status blocks archiving; mention, ticket and review-launcher links don't count.
  Not yet: restart-durable reports/wakes with stable report ids (item aa155e46; 68b601a8 left open for it).
  Release run: nextest 1174/1174 (engine + mcp), UI 2442/0.
- **Friction reporting + compact agent messages** (2026-11-04, `friction-reporting`, `compact-agent-messages`):
  `report_friction` / `list_friction` agent tools (Mast fills chat, harness, actual model, Space, item, branch;
  dedupe by fingerprint with occurrence counts), auto-capture (refused Mast tool calls, approval prompts + the
  answer, harness-rejected requests, turn errors) through the shared `mast_harness::redact` redactor, a
  Friction section in Space-root details (filters, expand, Create bug, Propose instruction, Dismiss, Export
  Markdown in the old agent-friction-log format) and an auto-capture toggle (default on). Transcript: messages
  from other chats, admin chats and Mast wakes render compact and muted with a source header (prefix-detected;
  structured sender metadata is a follow-up; relayed "Re:" answers are the user's own and render as human).
  Release run: nextest 2294/2294, UI 2442/0. Not yet: importing the old Cofactr friction log.
- **Neutral defaults 1B** (2026-11-04, `neutral-defaults-settings`, 3 Terra slices): one-time UI settings migration
  marker (existing vs fresh profile); fresh profiles get neutral defaults, existing ones keep their effective
  values: admin grant duration remembered (4 h), Team tab hide/auto/always (auto for both, matches today),
  compact single-chat rows (fresh off, existing on), sounds off / banners background-only / live Dock icon off
  / menu bar item off for fresh profiles. Orchestrator wakes default deliberately unchanged (already opt-in via
  starting workers). Rebindable outline/Team shortcuts split out (item 3b6f9919). UI 2436/0.
- **Neutral defaults 1A** (2026-11-04, `neutral-defaults-agent-text`): MCP instructions reworded neutrally (publish
  what you want reviewed; workers raise `kind: handoff`; finished workers can be archived), tool descriptions
  match the engine's real reach/archive rules, examples no longer use Mikey's process; per-Space editable PR
  review / self-review / fix-CI prompt templates (existing Spaces keep `/code-review` and `/fix-ci`, new Spaces
  get neutral prompts; Spaces arriving later via sync are migrated too, untested against a live remote);
  devcontainer paths only when the mount is verified. Release run: nextest 2279/2279, UI 2427/0.
- **Search fixes + coworker cleanup** (2026-11-04, `search-overlay-fixes`, `coworker-cleanup`): ⌘O/⇧⌘F inputs use
  ComposerInput (full editing); a per-checkout file index prewarms when a chat opens and follows the watcher
  (5 s repair in over-budget repos); ⌘O 300 ms → 13–16 ms per keystroke, indexed content search 13 ms first
  match. Follow-up: don't count gitignored dirs toward the watcher's 8,000-dir budget. Cleanup: upstream iOS
  (kept on `archive/ios`), landing, www-redirect, edge worker, workflows, Windows/Linux packaging and unlinked
  media removed; dev scripts → scripts/dev; brand test covers README + guides; `zeron update` says build from
  source; fixtures use /Users/example; internal docs moved to **docs/internal/** (this file is now
  `docs/internal/plans/work-outline/HANDOFF.md`). gitleaks: 40 history hits, all hashes/upstream test data/a
  public analytics key. Repo git auto-gc is off (`gc.auto 0`); run `git gc` manually when idle.
- **Typed Needs-you, phase 3** (2026-11-04, `needs-links-facts`, built in two small Terra slices after one
  stalled chat): `links[]` (artifact, file+line, diff/branch, PR, chat, https URL; validated for reach and
  shape) and `facts[]` (label/value, capped) on requests, persisted and in MCP create/read; card buttons use the
  existing handlers; review cards get a primary Open, handoff cards show Branch/Tests + Diff/PR; `detail`
  renders as Markdown and legacy text auto-links mast-artifact ids and PR URLs. Release run: nextest
  2269/2269, UI 2424/0. Phases 4–6 (inline answers + replaceOpen, handoff lifecycle, preview surfaces) not started.
- **PR review phase 4: walkthrough + review any diff** (2026-11-06, `review-targets`, `review-walkthrough`, Terra).
  Review targets are `ReviewTarget::Pr | Branch`. `review_publish` takes an optional `target`; branch anchors are
  validated against merge-base...head. Branch sidecars still carry a synthetic `pr` #0 for legacy readers; the UI
  reads `target`. Sidecar hunks use the shared `review_hunk_identity` (path + changed lines), the same as Changes;
  legacy sidecars are normalized on read (review_get and artifact reads), and unnormalizable ones show Outdated
  rather than range-matching. Review state is keyed by `target.state_key()` (`branch:<repo>#<branch>`). Opening a PR
  review inherits matching branch hunk marks, even with a cold status cache. `StartBranchReview` /
  `RpcClient::start_branch_review` launches a review chat on a fixed commit range (works for unpushed worker
  branches); it has no UI button yet (phase 4c). UI: a Review surface as a right-pane tab plus a full-width route
  sharing one view. Tour/free modes, step notes with risk, "N / M reviewed", `v` mark reviewed (manual only), and the
  2 s on-screen soft signal (faint, not counted). "Not in the tour" lists uncovered hunks. Keys n/p, ]/[, v, t are
  rebindable and excluded in inputs. Stale banner when the head moves. Known gaps: local or non-GitHub repos keep the
  old `ref:` key, so Changes marks aren't shared with the walkthrough there; remote-device forwarding untested.
  Release run: UI 2506/0. Mikey's decisions: keep the ordered explanation but never take the diff away; manual
  reviewed plus a soft signal; tab AND full-width; self-review and pre-PR worker review are a must.
- **Child-template proposal visibility** (2026-11-06, `child-template-proposal-visibility`, Terra): child-template
  proposals now also show on the container item, its template editor and the Space proposal list, with navigation
  to the container. When `ruleNodeId` is omitted it resolves to the node the proposer follows, or the template root
  for a direct child. If nothing resolves it errors, and legacy null-node proposals can't silently apply to 0 items.
- **PR review phase 3** (2026-11-04 overnight, `pr-review-search`): ⌘O quick open (fuzzy, recents, changed-in-diff
  group, `:line`), ⇧⌘F in-process streamed content search (ignore + grep-searcher; case/word/regex,
  include/exclude globs, caps, cancellation, only-changed-files), ⌘G go to line in files and diffs, file
  editor Replace moved to ⌥⌘F; all via the rebindable keymap. Back/forward history not built. Release run:
  nextest 2310/2310, UI 2420/0.
- **Typed Needs-you, phase 2** (2026-11-04 overnight, `needs-outcomes`): persisted `resolution.outcome`
  (approved, declined, changes_requested, chosen, answered, dismissed) + `values` + `via`; per-kind card
  buttons (approve → Approve/Decline, review/handoff → Approve/Request changes, Dismiss everywhere, note
  prompt); a fixed-format user-attributed relay to the requesting chat; admin bulk changes apply only on
  outcome `approved` (fixes "resolve with 'no' still approved"); `get_user_action_request`; outcomes in
  read_current. Release run: nextest 2253/2253, UI 2415/0.
- **Typed Needs-you, phase 1** (2026-11-03 night, `needs-kind-field`; design option C hybrid, plan
  mast-artifact:e0a05a3d0248e2e8d8d0fda6bb6214c4): Required Actions have `kind` (ask, review, approve, choose,
  handoff, notice) and `priority`; legacy text-only rows are classified on read; "Ready to merge" text
  matchers replaced by `kind == handoff` (status light, Team, archivable); producers set kinds (admin bulk →
  approve, PR review → review, plan questions → ask, option picks → choose); kind icons and groups;
  `notice` stays out of the badge/light; plan question cards show a link chip. MCP create takes kind/priority.
  Release run: nextest 2248/2249 (the 1 was a stale usage-text assertion, fixed), UI 2412/0. Phase 2
  (`needs-outcomes`, incl. the admin bulk "no still approves" fix) is in progress.
- **PR review phases 1 + 2** (2026-11-03 evening, `pr-review-read-diff`, `pr-review-sidecar`; Codex
  gpt-6.1-terra; brief mast-artifact:3e12f5d0be4cb1de57ed6b0f8be58992). Phase 1, Changes pane: filterable
  file sidebar (renames old → new, +/- counts), J/K next/prev hunk, ⇧J/⇧K next/prev file, hunk count,
  generated/lockfile auto-collapse, content-hash hunk identity, viewed state per PR (registry row; same-hash
  hunks stay viewed across head moves, cleared on PR close) or per diff target. Phase 2: `mast.review/v1`,
  `review_publish` (validates anchors against the real diff, lists valid hunks on error) / `review_get`,
  artifact kind `review`, stale-on-head-drift (local PR ref, live PR cache when it knows the head), summary
  + highlights card, review launcher prompt. Release run: nextest 2246/2247 (the 1 was a real
  review_get staleness bug, fixed in 663473a3), UI 2411/0. Phase 3 (quick open/search) and 4
  (walkthrough) next.
- **Surface stalled workers** (2026-11-03 evening, `surface-stalled-workers`): orchestrator wakes for a
  worker waiting on a prompt/approval (with the question text) and for a worker turn that ended after
  refused/failed Mast tool calls; both default on, toggles in `orchestrator-wakes.json` / Team tab.
  `get_chat`/`list_chats` expose the pending question and last-turn tool failures; markers in the Team
  tab and running popover; create_chat says which harnesses support Mast tools. Shipped on targeted
  tests only (engine orchestrator_wakes + agent_tools 60/60, MCP/proto 304, child's UI 2393); no
  overlap with the Codex MCP fix underneath.
- **Codex MCP approvals** (2026-11-03 evening, `codex-mcp-approvals`, installed 17:06): Codex chats in Ask
  mode had every `mast-agent` tool call silently denied (Codex sends `mcpServer/elicitation/request`
  before MCP tool calls; the harness answered "unsupported"). Now the injected `mast-agent` server gets
  `default_tools_approval_mode = "approve"` and its elicitations are accepted; other servers'
  elicitations (e.g. Computer Use) become a Yes/No prompt via the input bridge; rejected server requests
  log at INFO (`mast_harness::codex`). Harness 455/455, UI 2391/0. Verified live: a Codex Ask-mode chat
  made 5/5 Mast tool calls without prompts; Computer Use not yet exercised.
- **Release 2026-11-03 morning** (installed 11:32): `faster-releases` (nextest config with
  `release`/`quarantine` profiles, terminal-heavy tests capped at 2 at a time,
  `scripts/run_release_tests.sh`, `release-testing.md`; engine suite ~20 min vs 3.5 h+ projected for
  `cargo test`); `resolve-subtree-reach` (agents resolve requests on items in their own item's subtree,
  so a new coordinator can resolve its predecessor's children's actions); `compact-row-grant-admin`
  (Grant/Renew/Revoke admin on compact single-chat rows); `inherit-auto-approve` (Codex workspace-write
  adds the worktree git dir and the git common dir as `writableRoots`, verified against the real
  seatbelt sandbox, so commits/pushes don't prompt; `create_chat { autoApprove: true }` allowed only
  from a caller on Auto, Ask otherwise).
- **Archive finished workers** (2026-11-02, `archive-finished-workers`): shared rule in
  `engine/src/archivable.rs` (Team tab and agent tools): archivable = idle, no open Required Actions,
  and one of item completed / linked PR merged or closed / a resolved "Ready to merge" / idle > 3 days
  with the item completed or archived; returns a verdict + reason. Non-admin orchestrators archive their
  children (no rule) and workers under their project worker destination (rule, or `force: true` +
  reason, logged); admin chats archive any chat in their Space except themselves. Unarchive always
  allowed. In-memory agent archive log. Team tab "Archive finished (N)".
- **Known test failures** (pre-existing, not regressions):
  - Engine lib, on main `3801cb5b`: `capture_source_context_reads_the_dispatch_checkout`,
    `local_opening_tail_arrives_before_full_mirror_and_keeps_all_history`.
  - Engine lib, flaky while Zeron itself is running (likely the live instance lock):
    `instance_lock::holder_probe_reports_pid_without_disturbing_the_lock`.
  - Engine integration, flaky under parallel load (passes alone):
    `local_first::headless_sign_out_closes_joined_edge_rooms_and_stops_daemon`,
    `message_queue::restart_restores_the_queue_frozen_until_an_explicit_send` (sometimes passes).
  - Flaky under full parallel load (pass alone): engine `work_outline_rpc::mutations_stay_fast_on_a_large_local_registry`,
    harness `opencode::tests::stalled_prompt_post_has_a_bounded_timeout`.
  - Engine integration, flaky under load: `device_routing::workspace_file_surface_proxies_over_the_relay`.
  - Engine integration, flaky under full parallel load (passes alone 15/15, 2026-11-19): `orchestrator_wakes::hand_off_reclaims_a_queued_worker_report`
    (asserts the queue right after an async hand-off; fix item under Bugs).
  - Engine integration, flaky under full parallel load (pass alone): `artifacts::blocking_questions_own_actions_and_the_rest_roll_up_without_duplicates`
    (timing on the 2s question reconcile), `m5c_accounts_uploads_titles::cursor_login_flow_spawns_shim_and_auto_activates`.
  - Engine integration, flaky under full load (pass alone): `codex_login_resolver::codex_login_resolves_override_and_adds_its_directory_to_child_path`.
  - Engine integration, flaky under full load (pass alone): `instruction_delivery` revision-wait tests.
  - Flaky under full load with several child chats building (pass alone): `external_import_links_repair::repair_*`,
    `workspace_files::workspace_file_rpcs_list_search_read_write_and_watch`, harness `executable::tests::version_probe_bounds_hangs_and_rejects_nonzero`.
  - Engine integration, flaky under load (pass alone): `codex_login_resolution::{codex_login_honors_override_only_installation, login_child}`.
  - Load-sensitive (pass alone; hard timeouts): `pr_ticket_cache_login_shell` (5 s), `managed_accounts` sign-in tests (PTY), `pr_review_sync::setting_the_destination_syncs_the_space`, `m5_repos_diffs_terminals::git_status_stream…`.
  - Harness integration, environmental: `codex::models_discovers_visible_catalog_with_pagination`
    (expects `/bin/false`, which macOS lacks); `shell_env::falls_back_when_interactive_attempt_hangs`
    (flaky on the integration head too).
  - Engine integration, verified failing on `a3651a9d` before remove-canvas:
    - `e2e::generated_image_is_materialized_before_publication_and_survives_reopen`
    - `external_import_sync::sync_picks_up_a_closed_message_but_defers_the_new_open_reply`
    - `local_first::online_runtime_shutdown_stops_edge_workers_and_retires_the_graph`
    - `m5c_accounts_uploads_titles::titling_e2e_names_chat_and_renames_worktree_branch`
    - `message_queue::restart_restores_the_queue_frozen_until_an_explicit_send`

## Housekeeping (2026-10-25)

- All 80 feature worktrees under `.worktrees/zeron/` were removed and their local branches deleted.
- The 26 merged feature branches and the 193 copies of upstream branches were deleted from
  `origin`. Upstream's tags are still there, and so are ours (`pre-linear-integration`,
  `pre-canvas-removal`).
- Leftovers were saved to `~/zeron-dev-app-backups/leftover-patches-2026-10-25/`:
  - uncommitted edits found in the `work-outline-boundary-moves` and `pr-sync-create-fix`
    worktrees (both branches were already merged, so the edits are probably abandoned);
  - the unpushed `pr-sidebar-removal-work` commit, which `d7a30add` already contains.
- GitHub Actions are turned off on the GitHub repo. The workflows are upstream's release, deploy
  and TestFlight jobs, which would fail without upstream's secrets.
- The README (en and zh-CN) now describes the app (Creel then, Mast now), with the lockup from `docs/media/readme/`. The
  screenshot block is commented out until Mikey adds `outline.png`, `pr-page.png` and `plan.png`
  there. `.github/FUNDING.yml` is gone.
- `/Applications/Zeron.app` (upstream 0.2.59, bundle id `sh.zeron.app`) went to the Trash. Old
  backup binaries were pruned to the last three installs.

## Working model (since 2026-10-20)

Mast development is organised in the Work Outline under the root item "Zeron Dev"
(`48ef4b86c9967626ba1b6372e4cdfa66`, Space agent-mode-tools) with children Release, Bugs, Features,
Parked. A coordinator chat on that root starts child chats on the right items with
`create_chat { workItemId }`, polls `work_outline_read_subtree` for "Ready to merge" actions, and is
the only chat that merges, installs, edits this file, and pushes. The coordinator does the release
itself; there is no separate Release chat (Mikey, 2026-10-20). Feature and bug chats push their
branch and raise "Ready to merge: <branch>".

Coordinator tips:
- Never block on a child with a foreground `wait_for_turn`, because Mikey keeps giving new work
  while children run. A child that runs its tests in the background goes idle before it is done,
  so idle doesn't mean finished. Children also push early checkpoints, so a push doesn't mean
  finished either. In a background shell, watch the branch's sha on origin (`until git ls-remote
  origin <branch> | grep -q -v <last-sha>; do sleep 60; done`), and on each change read the subtree
  for the "Ready to merge" action. That action is the real done signal.
- After a release, delete the merged branch from `origin`, then remove its worktree and local
  branch.
- `work_outline_read_subtree` cuts long action `detail`s short (`detailTruncated`). For the full
  text, `read_chat` the `originChatId`.

The peon tray (`peon_tray.py`) was retired on 2026-10-20. Zeron's own banners and chimes replace it.

## In flight (snapshot 2026-11-24 evening; coordinator 9ca67830)

- **Installed: release 119** (646da66b5); Mikey to restart. 118 backed up as
  `~/zeron-dev-app-backups/mast-release118-56e798600`. Release tests vs 56e798600: nextest 2884/2884, UI 2940/0. Soak vs
  118: CPU 11.8 vs 13.4 %, steady 0.6 vs 5.3 %, footprint 547 vs 696 MB, growth −134 MB.
  - **UI driver medium and scale worlds** (feature/ui-driver-scale-worlds, Terra, two review rounds): `medium` and
    `scale` presets, the counts-only WorldShape (private file at the parent repo's `docs/private/ui-driver-shape.json`)
    and two medium scenarios (Tracker, Overview). Fake PR states come from the sandboxed gh stub through a driver-only
    engine feature that isn't in the installed graph.
  - **Privacy incident fixed:** driver runs (since at least 2026-11-23) started the external session importer.
    Scratch roots in /private/tmp held import cursors pointing at live ~/.claude transcripts, and some held
    databases. All 79 roots were deleted.
    - Root cause: MAST_SOAK suppressed auto-adoption but not the importer's repair and stale-sync jobs, and the
      driver sandbox allowed reads of account stores.
    - Fix: `MAST_DISABLE_EXTERNAL_IMPORT=1` is required by the driver and gates every importer entry point. The
      sandbox denies the literal and canonical paths of the account, credential and dev stores. Scratch roots are
      removed by a Drop guard and by `ui-drive stop` (killpg). The shape extractor uses a 0700 source and deletes it
      afterwards.
    - Friction 09227b52. Follow-ups: item f7ea7bce.
- **Next-gen app plan** (item 018c2b98, planner chat a818f204, plan mast-artifact:35d836f8, sheet mast-artifact:f12640a8,
  10 open rows): Michael approved the shell prototypes on 2026-11-24. Spec: `prototypes/next-gen/SPEC.md` in the parent repo
  (untracked, in .git/info/exclude). Items: A Tauri c35ac286, B zgx e7d7bfb7. Measured runs open windows, so they need
  Michael's go-ahead. Upstream's zgx move is parked under cae8f3ff.
- **Fork review:** a coworker's fork https://github.com/DSWallach/mast is being reviewed read-only (item a274efde).
- **Soak script template:** /tmp gets cleared. Recreate `soak-when-quiet.sh` from the 116 snapshot pattern (loop
  until load < 6, then perf-soak `--real` against /Applications/Mast.app, retry on 'stopping before').

## In flight (snapshot 2026-11-23 afternoon; coordinator 9ca67830)

- **Installed: release 118** (56e798600); Mikey to restart. 117 backed up as
  `~/zeron-dev-app-backups/mast-release117-3b66d9ad3`. Release tests vs 3b66d9ad3: nextest 2075/2075, UI 2940/0. Soak vs
  117: CPU 9.8 vs 8.8 % (machine busier; under the 1.5× gate), steady 0.6 vs 0.6 %, footprint 559 vs 602 MB, growth −37 MB.
  - **Context window fix** (fix/context-window, Terra):
    - The 200K/1M option is gone for 1M-native Claude models (`catalog::native_million_context`). A saved
      `contextWindow` on those models is ignored.
    - Overview and sidebar meters divide by the recorded window (200k only as the fallback).
    - The composer's context indicator has its own gated observer, and the hover card reads the same value.
    - Unresolved: Mikey saw a 36% ring while the card said 183,925 / 1,000,000 (18%). It isn't staleness, and
      modelUsage picks the main model. Ask for a screenshot of both together if it recurs.

## In flight (snapshot 2026-11-23 11:50; coordinator 9ca67830)

- **Installed: release 117** (3b66d9ad3) at ~11:50; Mikey to restart. 116 backed up as
  `~/zeron-dev-app-backups/mast-release116-c256f2f12`. Release tests vs c256f2f12: nextest 2867/2867, UI 2938/0; soak
  vs 116 CPU 6.1 vs 6.0 %, steady 0.6 vs 0.6 %, git/min 161 vs 161, footprint 514 vs 640 MB, growth 58 MB.
  - **Tracker stage fix** (fix/tracker-stage, Sonnet, four review rounds). Mikey: tickets showed Building while in PR.
    - Root causes: an expired PR cache entry returned None; a failed gh lookup erased a known PR for 10 min; the UI
      didn't refetch chat link statuses on PR-cache changes.
    - Now: fetchers distinguish not-found from transient errors (logged-out gh is an error, never not-found).
      Transient errors keep the data and its original fetched_at, with backoff, for up to 24 h (merged exempt).
    - Revisions bump only on a real change and carry the changed URLs and branch keys; only the matching chats expire
      into an "owed" set that the top-up draws from (max 8 in flight). No polling; replies are batched into one
      rebuild per 200 ms.
    - Merged URL entries are never re-queued; branch keys still are, so reused branches update. Pending runs oldest
      first.
  - **Go-to-definition** (Terra, three review rounds).
    - F12, an alternate shortcut and ⌘-click; a picker for several matches; back/forward history; pop-outs reveal
      same-file definitions locally.
    - One shared symbol watch per checkout, dropped after 2 min idle; a cold ⌘-click parses on demand.
    - Installed graph baseline +1 edge: mast-ui → lsp-types (already in the graph via gpui-base).
  - **Friction from all Spaces** (Terra, three review rounds).
    - A per-Space grant, default off, set by the human only (`SET_FRICTION_READ_GRANT` is in HUMAN_ONLY_METHODS).
      `list_friction scope:"allSpaces"`.
    - Foreign rows are filtered to Mast-qualified tool names (`mcp__mast-agent__…`, `mast-agent.…`), Auto
      refused-call summaries, or Mast GitHub links. They're stripped to 10 fields, secret-redacted (agent reports are
      now redacted at write time too) and read-only.
    - Not fixed (RPC security item 3196df1d): the raw `ListFrictionReports` over the localhost port still takes any
      space_id.
- **Release-run gotcha:** run `scripts/run_release_tests.sh <last installed sha>` when you've already pushed; with no
  arg it diffs against origin and selects nothing.
- **Parked 2026-11-20:** upstream moved its desktop app off GPUI to zgx (#1499); evaluation item cae8f3ff under Parked.
- **Worker wakes:** worker reports after a Mast restart can be lost: two workers sat in "Reconnecting n/5" for 2 h
  (friction 9d648a3a), and a finished round on 11-20 never woke the coordinator. After a restart, check each worker
  with read_chat. When waiting on a push, poll the branch with `git ls-remote`.
- **Next:** Wave 4 (container b8ccbcd9); follow-ups (pop-out part 2 cf4d9687, scope-map polish 9b626d42, flaky tests
  8e9406cb / 82fe9f5f).

## In flight (snapshot 2026-11-19 20:40; coordinator 9ca67830)

- **Installed: release 116** (c256f2f12) at 20:37; Mikey to restart. 115 backed up as
  `~/zeron-dev-app-backups/mast-release115-aa41a65ff`. Full perf gate: release tests 2839/2839 + UI 2917/0; soak vs 115
  CPU 5.7 vs 7.1 %, steady 0.6 vs 0.8 %, git/min 145 vs 139, footprint 554 vs 610 MB, growth 52 MB.
  - Contents beyond the 17:20 list: **startup CPU fix** (perf/provenance-tips, Terra, one review round + hardening):
    provenance ref tips and parsed packed-refs are cached per canonical common git dir instead of per worktree (≈35
    worktrees share the Mast repo; any ref change re-read every one), inode in packed-refs keys, per-checkout prior_tips
    unchanged (recording-listener test). A 10 s startup profile had shown 11.5k samples in `read_ref_tips`.
  - **Add to chat fix** (fix/add-to-chat-target, Terra, two review rounds): Add to chat stages on the chat on screen,
    not the chat the shared file tab was opened from (Mikey hit this); visible errors/confirmation; ⌘⇧L bound in Files
    and falls through to Open Outline with no selection. Covers the queued "Add-to-chat shortcut" item.
  - Quarantined `provenance_budget::blame_cache_hit_and_idle_stay_within_the_git_budget` (fails ~1 in 3 on cb833c26c;
    Bugs item 82fe9f5f). `acp::pi_interrupt_kills_tool_process_group` failed once at load 19 (a VM running), 6/6 alone.
  - Two engine tests fail only from a `.worktrees/` checkout, pass in the main checkout (noted on 82fe9f5f).
- **Profiling startup:** `/private/tmp/r116/startup-soak.sh` is perf-soak with the sample at 10 s for 10 s and no stamp
  (`--template <dir> --seconds 70 --order ab`). Useful when a soak fails on whole-run CPU but steady CPU is fine.
- **Next:** resume go-to-definition (e5484b6c) and review its fix round; friction from all Spaces (4aa4cc1c); wave 4.

## In flight (snapshot 2026-11-19 17:20; coordinator d7779d53 handing off)

- **Mikey restarted at 17:05 onto release 115** (installed aa41a65ff). User guide for 112–116:
  mast-artifact:d87cfddf154ea0db4cd247b65ea244e9.
- **Release 116 NOT installed yet** (HEAD 858410d36 at the release run; integration head may be ahead).
  - Contents: re-review (phase 7), provenance + blame, friction fixes (agents complete items with their own handoff
    open; chat start activates pending items; long Required Action detail truncates), parked CoW-worktree proposal.
  - Release tests passed on the rerun, after quarantining `orchestrator_wakes::hand_off_reclaims_a_queued_worker_report`
    (flaky under load, passes alone 15/15; fix item under Bugs).
  - Soak run 1 FAILED on CPU: cand2 steady 14.5 % vs 0.7–0.9 %, busy in `diff_sync::read_head`, under load average
    8–11; cand1 was normal (0.8 %). A rerun was in flight at handoff: `/private/tmp/r116/soak-when-quiet.sh` (waits
    for load < 6), log `/private/tmp/r116/soak.log`, run 1 in `soak-run1.log`.
  - If the rerun fails again: look for what calls `read_head` repeatedly. Provenance's `note_head` runs on every
    snapshot of a bound checkout; check how often snapshots happen with real data. Fix before installing.
  - If it passes: install per "How we work", then push.
- **Running:** go-to-definition (Terra, chat e5484b6c, branch feature/goto-definition, worktree `.worktrees/zeron/
  goto-definition`). Review-fix round 2 is committed through 2fed29fc7 but unverified; it is paused ("no cargo until
  resume") for the release. Send "resume" after 116 installs, then review it (fixes 1–9 from the first review).
- **Queued, in order:**
  - Mast Dev reads friction from all Spaces, opt-in grant (item 4aa4cc1c). Mikey wants this, because his Cofactr-Space
    friction (e.g. a review_publish failure in a PR review) is invisible to Mast Dev today.
  - Add-to-chat shortcut (⌘⇧L, never bound).
  - Wave 4 (container b8ccbcd9): right-pane split, diagrams tied to code, dependency overlay, opt-in LSP.
  - Follow-ups: pop-out part 2 (cf4d9687), scope-map polish (9b626d42), the flaky test (8e9406cb).
- **Open for Mikey:** ⇧⌘O → symbol search (move Overview to ⌥⌘O; request a62da6b9, blocks completing the symbol-index
  item); approval rules for `cargo test` and ui-preview in worktrees (they would remove most of the ~100 approval wakes
  a day); "Keeping the Tracker clean" instruction; iOS research; pinned tabs A; UI driver plan questions.
- **Outline cleanup done 2026-11-19:** closed 12 old research and design items (Mikey's OK) and completed merged
  items. **Coordinator rule:** complete the work item yourself when you merge, until 116's fix is installed.
- **Lessons this run:**
  - Every feature slice needed 2–5 independent Opus review rounds; budget for them. Workers inverted a requirement
    once (re-review removals); verify each fix, not just the tests.
  - Terra stalls on open-ended feature briefs; give a numbered checklist with decisions made, or use Sonnet.
  - Release runs: `/private/tmp/release-when-quiet.sh <dir>`, then a soak with the `soak-when-quiet.sh` pattern.
    Check for `^RESULT:`.
  - Merging after other features landed causes conflicts in changes.rs, file_list.rs, keymap.rs and snapshot.rs. Keep
    both sides, then `cargo check -p mast-ui --tests`. Don't chain cleanup after `git merge` with `;`: it ran after a
    failed merge once.

## In flight (snapshot 2026-11-19 16:30; coordinator d7779d53)

- **Installed: release 115** (aa41a65ff), full perf gate (soak vs 114: CPU 6.1 vs 6.2 %, git/min 126 vs 142, footprint
  554 vs 507 MB, growth 13 MB). Contents:
  - **Scope map** (Terra, two review rounds): folder treemap of a diff in Changes, Map/List toggle (default List),
    folder rollup bars, min tile size + ≤300 tiles with "N more" groups, keyboard nav, risk outlines from the
    walkthrough. Low-severity polish logged (item 9b626d42).
  - **Pop-out windows** (Sonnet, three review rounds): ⌥⌘N / tab menu "Open in new window" for File, Diff/History and
    Subagent; light PopoutWindow root (spike: docs/internal/plans/2026-11-19-popout-windows-spike.md); every exit path
    gates on pop-out unsaved files; ref-counted subagent watches. Chat, Tracker, Item, Review, Artifact, Browser,
    Terminal pop-outs deferred (item cf4d9687).
- In progress: review phase 7 (final round: removal detection per changed-line run), provenance + blame (fix round:
  attribution only for a chat's own first-parent commits during its turns; first writer wins). Queued: go-to-definition,
  then wave 4.
- Release runs: `/private/tmp/release-when-quiet.sh <dir>` waits for worktree cargo to stop, then builds + tests.

## In flight (snapshot 2026-11-19 14:30; coordinator d7779d53)

- **Installed: release 114** (c68a6d151), full perf gate (soak vs 113: CPU 6.0 vs 5.7 %, footprint 510 vs 513 MB,
  git/min 127 vs 123, within run-to-run noise). Code review & navigation waves 2–4 (Mikey: "make all of them",
  2026-11-18; container item b8ccbcd9, plan mast-artifact:060317a6):
  - **Review phase 6** (Sonnet after Terra stalled): "Submit review…" posts one batched review via the user's `gh`,
    only on a click. SubmitReview/PrepareReviewSubmit are refused on the localhost WebSocket port and relay
    (HumanOnlyGate, engine/src/human_only.rs); placement against GitHub's patch with ±2 context lines; a persisted
    per-target nonce + saved comments (with body hashes) prevent double posts across lost responses, timeouts,
    restarts and edits. Four review rounds. Untested against real GitHub: Mikey's first live post is the check.
  - **Symbol index** (Terra): engine-owned tree-sitter index started on first use (LRU 2 checkouts, idle retire),
    query RPC (≤200, truncated flag), symbol picker on **⌥⇧⌘O provisional** (Mikey to decide ⇧⌘O swap with
    Overview, request a62da6b9), outline, ⌘F in code, back/forward (keymap), per-file history following renames.
    Two review rounds plus coordinator follow-ups (ffe95781e).
  - Installed-graph baseline updated (engine → mast-syntax; no new crates); the first release run failed only on that.
- **Security follow-up logged** (Bugs, 3196df1d): the engine's localhost RPC port is unauthenticated and agents get
  it; any agent shell can call any RPC. Only Submit review is gated so far. Needs a design pass.
- In progress: scope map (Terra, fix round after review). Queued: pop-out windows spike, provenance + blame, review
  phase 7, go-to-definition, then wave 4 (right-pane split, diagrams tied to code, dependency overlay, opt-in LSP).
- Mikey's running Mast predates 112; one restart picks up 112–114.

## In flight (snapshot 2026-11-18 17:05; coordinator d7779d53)

- **Installed: release 113** (2a24619b4), full perf gate (soak vs 112: CPU 6.0 vs 5.8 %, footprint 533 vs 572 MB,
  git flat; UI-only change, so the engine soak is noise). C1.5: `render_row` 456 → 121 and `render_tool_group`
  617 → 81 lines (row content/metadata, tool-group frame/header/chips); render characterization in
  `transcript/render_tests.rs`. **Refactor Phase C1 is complete** (slices 1–5 in 112–113).
- First 113 soak aborted itself on load 12.4 (Docker VM); `/private/tmp/r113/soak-when-quiet.sh` pattern (wait for
  5-min load < 8, retry on abort) got a clean run. Watch for `^RESULT:` in release logs, not `RESULT` (the scenarios
  line contains the word).
- Mikey is on 110 until he restarts; one restart picks up 111–113.
- Next: perf candidates in mast-artifact:16024d9d2e87924e0e513ee5a731308e (measure first), or the next refactor phase.

## In flight (snapshot 2026-11-18 15:50; coordinator d7779d53)

- **Installed: release 112** (68de4092a), full perf gate (soak vs 111: CPU 5.5 vs 5.7 %, footprint 448 vs 538 MB,
  git flat). Refactor Phase C1 slices 1–4 (design mast-artifact:4bde59e77d7c0f96bd529dbc7969ffcf), each with
  characterization tests committed first and an independent equivalence review:
  - C1.1 `Shell::render` 767 → ~380 lines (`shell/frame_state.rs`, `shell/action_bindings.rs`).
  - C1.2 `Composer::send` 902 → 34 lines (`composer/send.rs`; `#[cfg(test)]`-only pre-dispatch failure hook).
  - C1.3 ACP `run_session` 1,203 → 359 lines (`acp/session.rs`, `AcpSessionState`, per-arm methods).
  - C1.4 `Transcript::sync` 562 → 12 lines (`transcript/sync.rs`, tests in `transcript/sync_tests.rs`).
- First release run had two diff_sync timeouts (attach_reprobe, repair_backoff) while fseventsd sat at ~200 % right
  after deleting three worktree `target/` dirs; both passed alone and the full rerun passed (non-UI 9 min vs 31).
  **Lesson:** delete worktree targets after the release run, or wait for fseventsd to settle before starting one.
- Mikey is on 110 until he restarts; one restart picks up 111 and 112.
- Next: C1 slice 5 (transcript row and tool-group rendering). Open for Mikey: proposed approval rules (ui-preview;
  `cargo test` outside the sandbox in worktrees, which would stop ~15 ACP-worker approval wakes per slice).

## In flight (snapshot 2026-11-18 11:00; coordinator 544369c6 handing off)

- **Installed: release 111** (2c02e2c3c), full perf gate (soak vs 110: CPU 4.7 vs 4.7 %, footprint 458 vs 526 MB).
  tracker-polish from Mikey's real-data audit: stage pips success/current/muted (Not started shows none); merged
  PRs' Ready-for-review requests move to cleanup ("Resolve stale request"); local items titled with a ticket id
  merge into that Linear row or get a "Link ENG-n" cleanup entry; probable duplicate hints; "N chats" label size.
- Mikey restarted onto 110 at ~09:15; post-restart sample healthy (main ~8 % busy, footprint 1.7 GB).
- Next: refactor Phase C1 slices (design mast-artifact:4bde59e77d7c0f96bd529dbc7969ffcf).

## In flight (snapshot 2026-11-18 09:00; coordinator 544369c6, overnight run done)

- **Installed: release 110** (85746ce8a), full perf gate (soak vs 109: CPU 4.8 vs 4.8 %, footprint 539 vs 613 MB,
  git flat). Contents: composer.rs (8,979 → 3,204), overview.rs (7,017 → 2,041) and shell/spaces.rs (5,288 → 717)
  pure-move splits; orchestrator_wakes crash-recovery test isolated; cleanup pass 3 (clippy 1102 → 1068, all targets).
- Mikey is still running 105 (restart picks up 106–110).
- Next: refactor Phase C1 (design mast-artifact:4bde59e77d7c0f96bd529dbc7969ffcf): 1 Shell render reconciliation/actions,
  2 Composer::send, 3 ACP run_session, 4 Transcript::sync, 5 transcript row/tool rendering. Open for Mikey:
  "Keeping the Tracker clean" instruction proposal, ui-preview approval rule, older items.

## In flight (snapshot 2026-11-18 07:40; coordinator 544369c6, overnight run)

- **Installed: release 109** (2053cec30), full perf gate (soak vs 108 equal: CPU 5.1 vs 5.0 %, footprint 620 vs 614 MB,
  git identical). Contents: proven pure-move splits of shell.rs (12,770 → 4,156), rpc.rs (forwarding policy and
  transcript streams moved; new `every_rpc_method_has_a_route_and_consistent_forwarding_policy` test),
  changes.rs (7,532 → 2,338) and transcript.rs (9,111 → 4,803, 14 modules).
- Merged for 110: composer.rs (8,979 → 3,204) and overview.rs (7,017 → 2,041) splits.
- **Release runs need a quiet machine:** compiles saturate fseventsd and the diff_sync_* tests time out. Pause worker
  cargo before release tests and soaks. Release 109 also hit a rare `orchestrator_wakes
  discarded_wake_row_is_reclaimed_again_after_crash_and_unarchive` flake (item d01e2493, fix queued).

## In flight (snapshot 2026-11-18 morning; coordinator 544369c6, overnight run)

- **Installed: release 108** (62f242389), full perf gate. Soak vs 107: CPU 4.9 vs 5.1 %, footprint 468 vs 586 MB,
  git flat. Contents: worker-hygiene (three opt-in per-Space automations, all off by default: own worktree + ticket
  branch for new workers, complete item on PR merge Off/Ask/Automatic with a 3-min bounded scan, auto-archive
  finished workers after N days on a daily sweep), style-token-pass (shared controls and tokens across Changes, code
  refs, Tracker, diagram card; Horizon one-surface; "area" ids are Horizon aliases), flaky-diff-sync (completion
  signals, explicit mtimes), refactor-state-split (state.rs 4,462 → 677 lines, 10 modules, proven pure move),
  benches for transcript sync and Changes widths (both measured fine, no code change).
- Merged for 109: refactor-shell-split (shell.rs 12,770 → 4,156 lines, proven pure move).
- In progress: refactor B10 (rpc.rs routing-completeness test, then pure moves).

## In flight (snapshot 2026-11-18 03:45; coordinator 544369c6, overnight run)

- **Installed: release 107** (1929473e1), full perf gate. Soak vs 106: CPU 4.7 vs 4.9 %, footprint 562 vs 665 MB,
  git/min 117 vs 113 (noise). Contents:
  - tracker-lifecycle: tickets grouped by derived stage (Not started → Building → Code review → PR review → Ready
    to merge → Done) from a linked Linear project (completed included, per-Space account, capped at 250 with a
    "more not loaded" note) or local child items; cleanup list (collapsed) + `work_outline_health` MCP tool;
    `work_outline_link linearProject` and a "Link Linear project" picker.
  - board-removal (Board/Table gone, −1,655 lines; cached worker chips on outline rows open the Tracker or the
    item's details).
  - review-fixes (Linear sync writes fenced on account change; reused-chat delivery mode chosen atomically;
    create_chat validates before claiming its idempotency key; registry allows agent Linear-project links only in
    the caller's subtree).
  - perf: session publication (coalesced Working updates, incremental merged state) and sidebar rows (hover ≤2 row
    rebuilds per move, 930→59 over 30 moves; no per-row chat scans; 2.4 M→272 k session comparisons).
  - tests: flaky-harness, flaky-instruction-delivery, flaky-queue-stream (7 tests back from quarantine; tokio
    `test-util` dev-only in mast-engine); history search: an abbreviated SHA returns only prefix matches.
  - docs-refresh (README, guide, mcp.md, pr-review.md, developer docs).
- Measured and dropped (no code change): transcript incremental rows (0.52 ms p95 sync at 17k rows), shared FSEvents
  stream (27–102 s vs 1.3 s; diff-sync settle slowness is machine load). Reports: perf audit
  mast-artifact:16024d9d2e87924e0e513ee5a731308e, flaky tests mast-artifact:f9655ec82a1be489408e37123f80b07b, reviews
  mast-artifact:e4aa3065599c9345432c3a0f9b0c6eec and mast-artifact:107858f99f6c7689b8604386717c32f8.
- Standing OK from Mikey (2026-11-18): perf, refactor and cleanup work without asking; gate still applies.
- In progress: style-token-pass (tokens across Changes, code refs, Tracker, diagram card). Queued: worker hygiene
  (opt-in), flaky build 3 (diff-sync readiness), perf Changes widths (#2/F1) and diff sidecar (#4), refactor Phase B
  (B10 rpc routing test first), cleanup pass 3, sidebar ordered-model cache + `session_for` index.

## In flight (snapshot 2026-11-17 22:30; coordinator 544369c6)

- **Installed: release 106** (a34b03500), full perf gate. Soak vs 105: CPU 4.5 vs 5.0 %, footprint 474 vs 524 MB,
  git/min 102 vs 114. Contents: linear-accounts (named Keychain accounts, per-Space `linearAccountId`),
  tracker-cleanup (one row per leaf item, stages from item PR refs, Other chats/Stale groups, clean PR/activity
  lines), orchestrator-guardrails + fixes (create_chat reuses a live chat on the item unless `newChat: true`;
  `linearIssue` on create_child; `work_outline_link` for PRs/issues scoped to own subtree; ticket-based default chat
  titles; replay fingerprint uses parsed inputs), diff-sync-attach (attach timing spans under `diff_sync::attach`,
  recent-activity-first watch setup with FIFO turns, `is_attached(cwd)`).
- Measured: 50-checkout watch attach settles in ~49 s; watch() p50 0.96 s / p95 6.7 s (FSEvents startup), queue wait
  p50 35 s behind the 2-permit gate. Next build: one shared batched FSEvents stream (needs a native wrapper; notify 7
  restarts its stream per added path). Report: mast-artifact:a492f187664f12d92ebedacb3b85fb37.
- Release-test env: `diff_sync_*` binaries now run one at a time (nextest `fsevents-heavy` group, 5-min limit);
  in parallel they saturated fseventsd. Release tests passed with that.
- **Building:** lifecycle Tracker (item 865a0a53, design approved, pinned plan mast-artifact:28df5ed9b3db8d1127e8495ce2604552):
  six derived stages, Linear project (completed included, per-Space account) or local work-item tickets, grouped
  view, cleanup list + `work_outline_health` MCP tool, `work_outline_link` linearProject + "Link Linear project".
- Open for Mikey: instruction proposal "Keeping the Tracker clean"; ui-preview approval rule.

## In flight (snapshot 2026-11-17 18:00; coordinator 544369c6)

- **Installed: release 105** (9a07243be; = 104 + changes-polish, code-ref-moved-state, tracker-review-and-data
  [ticket-first Tracker rows per the mockup, Review PR/Merge, head-checked review progress, PR +/−, ticket titles,
  engine-side checklist counts on Session.todo_progress]). Installed with `--skip-perf-gate` (Mikey's OK): release
  tests failed only on environment load (a VM at ~290 % CPU, fseventsd ~100 %). Three diff_sync startup-settle tests
  (watch attach for 50 checkouts: 46–48 s alone at 104, 54–56 s at 105) and a 2 s harness read timeout; each passes
  alone. Soak vs 104 passed. Override logged in docs/internal/perf-gate-overrides.md. Hang guard widened in
  diff_sync_repair_backoff (566b877ed) + 5-min nextest limit (9a07243be).
- **Follow-up:** watch attach ≈1 s per checkout at startup is slow in itself (and ~15 % slower at 105 in the A/B,
  cause not found: 105's engine diff is session todo_progress + PR size only). Worth a look on real data.
- **Ready, held for 106:** linear-accounts (6be5f727c): named Linear accounts (Keychain, existing `api-key` = Default
  read in place), per-Space `linearAccountId` (absent = Default, no migration), account-scoped sync/queries/lookups,
  Settings → Linear accounts + per-Space picker.
- **Next (item 865a0a53):** Tracker as ticket lifecycle. Rows from the linked Linear project (read-only, Space's
  account) or, without Linear, the item's child work items; stages derived from facts: Not started → Building →
  Code review → PR review → Ready to merge → Done; Needs-you as a badge. Waiting on Mikey's OK of the stages.
  No local ticket IDs at first.
- Mikey's Mast restarted 15:27 onto 104. Post-restart sample at ~4 min: CPU 14–31 % settling, footprint ~2.2 GB.
  perf-sample.py now counts `semaphore_timedwait_trap` as idle (the freeze watchdog showed as busy).

## In flight (snapshot 2026-11-17 evening; coordinator 544369c6)

- **Installed: release 104** (4bbb92ba0; = 103 + code-block-and-chips + tracker-surface + crash fix). Perf gate:
  release tests pass; soak vs 103 equal CPU (5.2 vs 4.9 %), footprint 452 vs 564 MB, git/min 108 vs 100 (noise
  across ABBA runs 94–122). Mikey restarts to pick it up (he was on 96).
- **Design pass** (item 7d000a94, Sonnet): readability design mast-artifact:bf3fbf6160ebe4be8e1dcbe9285484d6 and
  work-item tracker design mast-artifact:fce1272a689cfd2b71ee0be36e30d5c9. Mikey: drop Board/Table; wants one view
  per work item/project tracking workers, tickets, PRs and progress, plus code review. Option A approved: the Team tab
  becomes the Tracker. Design files on disk (workers can read them): `<home>/Projects/agent-mode-tools/.mast/
  artifacts/design-wave-1-surfaces-readability-pass-syntax-c-7d000a94/*.fragment.html`.
- In 104: shared CodeBlock (components/code_block.rs), highlighted pinned code-reference view (header, gutter,
  Changed badge/banner, Show diff vs current), chip badges + hover/focus cards; Tracker slice 1 (rollup pills,
  grouped rows, six-step fact-only stage track, expand). First 104 test run caught a crash: reading
  `ListState::logical_scroll_top()` inside a transcript row panics (RefCell already borrowed); fixed in c60ef6c4e.
  Rule for briefs: never touch a ListState from row/render callbacks.
- Ready, held for 105: code-ref-moved-state (a3ff14fff, moved references show the pinned snippet + reanchor action).
- Building (Codex terra): tracker-review-and-data (Tracker slices 2–3: Review PR/Merge/Tour badge, checklist count,
  PR +/−, ticket titles) and changes-polish (Changes panel polish + diagram thumbnail). Left after those: Tracker slice
  4 (outline rollup chip, delete Board/Table ~1900 lines) and readability slice 4 (tokens/style pass).
- perf-soak.sh guard fixed (5a617a47d): it refused to start when pgrep matched its own subshells or an agent's
  wrapping shell.

## In flight (snapshot 2026-11-17 09:30; coordinator da545eee handing off)

- **Installed: release 103.** Mikey is still on 96 until he restarts. Nothing is building; no workers are running.
- **Next, Mikey's ask: a design pass** on the wave-1 surfaces. He finds code hard to read: code shows without
  syntax colour or formatting (e.g. the code-reference file view and its snippets/chips render plain text; check
  diffs, review drafts, transcript code blocks too). Screenshots (offscreen, release 103):
  mast-artifact:be0bb4e213364d721971b1f328db7928 (code reference), dbf6e9b4bb78fe69db23cd670a3b7245 (word diff),
  898b17235424ac5e05a36bce5b2e8d70 (review drafts), 6d12d5360d60446f988b69b8579d329b (board),
  dbeeaa997abd06365309848e299646cf (table), e5011e217e8bbaf6818fd0740be39996 (diagram card). Known nits: table
  "Answer" button has no padding; board cards have an empty expand-arrow row; "Review" on in-progress cards is
  unclear; dark diagram thumbnail sits on a lighter panel. Render with `scripts/ui-preview.sh --screens
  code-reference,changes-expanded,walkthrough,board,board-table,diagram-card` (set MAST_PREVIEW_CODE_REF=1 and
  MAST_SNAPSHOT_POPULATED_CHANGES=1), and the UI driver (`scripts/ui-drive`) for flows.
- Tooling now available: headless UI driver with YAML scenarios (crates/ui-driver, scripts/ui-drive), enforced perf
  gate on install, perf-soak with ABBA order. Xcode git hangs on `git submodule`: use
  `DEVELOPER_DIR=/Library/Developer/CommandLineTools` until Mikey opens Xcode or reboots.

## In flight (snapshot 2026-11-17 07:00; coordinator da545eee, overnight run)

- **Installed: release 103** (16afa8cf7, 08:16; = 102 + UI driver YAML scenarios + cleanup pass 2). Mikey's running Mast is still 96 (up since 2026-11-16 17:20); one restart picks
  up 97–102. Every install went through the enforced perf gate (release-test + soak stamps).
- Overnight releases: 99 (sidebar spinner rows cached 2268→264 builds/30 frames; freeze watchdog writes
  `freeze-*.txt` next to mast-headed.log on a ≥2 s main-thread stall, `MAST_FREEZE_WATCHDOG=0` disables; Add to chat
  on submodule hunks via CodeRef `repoRoot`), 100 (UI driver B4/B5, cleanup pass 1: clippy 743→650 and the last
  direct spawns through mast_harness, engine startup pass: first sync 6→4 git per checkout, indexed artifact
  watcher, no-PR cache 10 min), 101 (refactor Phase A batches 1–3: ~43k lines of inline tests/panels moved out of
  the biggest files as proven pure moves; UI perf: cached transcript view 4.1→1.0 ms per keystroke, Changes
  active_diff by reference, transcript sync scans only changed rows, cached diff font advance), 102 (repair-tick
  backoff for quiet watched checkouts, missed-change bound 600 s; ABBA soak order).
- **Headless UI driver** (plan mast-artifact:10cf07e3bdb339761d5962e11b1ea3f8): B0–B5 done. crates/ui-driver
  (test-only, `scripts/check-installed-clean.sh` in every release run keeps it out of the installed binary),
  GPUI from `Pluto8195/zui` branch `mast/test-support` (one test-support commit on gaelcado/zui@53869c2),
  `scripts/ui-drive start|tree|click|type|key|png|stop` in a sandboxed seeded world, first flow test
  `first_flow_send_approve_finish`. Next: scenario/batch mode, medium and Mikey-scale presets, cue-driven fake agent.
- **Environment:** since 2026-11-16 evening Xcode.app's git hangs on shell-script subcommands (`git submodule …`
  stalls at exec). Release runs and workers use `DEVELOPER_DIR=/Library/Developer/CommandLineTools`. Mast's
  production code doesn't call `git submodule`. Opening Xcode once or a reboot probably clears it.
- After 102 (integration, not installed): UI driver YAML scenarios (`scripts/ui-drive run <file>.yaml`, 3 scenarios,
  report-only in release runs) and cleanup pass 2 (clippy 650 → 517). Release 103's first gate failed only on the
  test-only `world_template same_seed_same_hash…` driver flake (now quarantined, item 85e3292a); the rerun passed
  (soak vs 102: equal CPU and git rate) and 103 is installed.
- **Needs Mikey:** refactor plan Phase B/C review (mast-artifact:2ee3c40d3a47f5d2cda34f8afeee43c5); Perf gate
  instruction proposal; approval rule for niced cargo commands; review phase 6 (GitHub posting) call; older items.
- Follow-ups: Changes width shaping off the render thread (changes widths; needs design); transcript sync still
  clones new_rows; result_large_err clippy (374) needs an error-boxing design; soak memory noise ~55 MB.

## In flight (snapshot 2026-11-16 20:35; coordinator da545eee)

- **Installed: release 98** (84cfe6629), the first install through the enforced perf gate (release-test + soak stamps).
  Mikey's running Mast is still 96 until he restarts; 97 was installed then superseded by 98 before a restart.
  - 97: idle git rate fix (spawn-free checkout identity, repair probe re-baselined, diff_sync/diff_targets under the
    gate) + attach-kick race fix. Soak vs 96: idle git 50 vs 200/min, idle CPU 0.7 vs 3.0%, footprint 390 vs 645 MB.
  - 98: wave 1 merged from `code-review-nav`: code reference + Add to chat (hidden on expanded submodule hunks and
    deleted-only Latest Turn hunks; follow-up 0527213f), coordination board + table, inline diagrams (thumbnails on,
    auto-open off by default), word diff + expand context, review phase 5 (local drafts, suggestions, read-only GitHub
    threads; no posting). Release tests 2584/2584 + UI; soak vs 97 equal CPU and git rate, footprint 456 vs 347 MB.
- Gate fixes on the way: quarantined the two mast-preview peer/leak tests that always hang on this Mac (still run
  under `--quarantine`); perf-soak reads footprint via proc_pid_rusage (vmmap stalled the sampler 10+ min) and
  invalidates runs whose window overruns; it refuses to start at load > 12.
- Next: headless UI driver phase 1 (plan mast-artifact:10cf07e3bdb339761d5962e11b1ea3f8, items B0–B5, agent-only,
  nothing in the installed binary). Then code-ref submodule hunks (0527213f), review phase 6 (needs Mikey's GitHub
  posting call), sidebar spinner redraw (2694931e), freeze watchdog (17c9d278).
- Needs Mikey: Perf gate instruction proposal; approval rule for niced cargo commands; older items.

## In flight (snapshot 2026-11-16 evening; coordinator da545eee)

- **Installed: release 96** = 95 + network git ops skip the spawn gate + post-mortem guardrails P1–P4
  (`chore/perf-guardrails`: `mast_harness::backoff`, clippy `disallowed-methods` for raw `Command::new` with 28
  non-git/gh sites still allowed under `TODO(post-mortem P3)`, git spawn-rate `warn` above 120/min with top callers,
  idle-budget tests). Release tests since 94 (e8d628596): UI 2651/0; nextest 1865/1867, the known
  `preview::leak remote_preview_churn` hang plus `idle_spawn_budget` over by one (87/86); its budget now counts
  miss refreshes per 3 s window, 5/5 green. 95 at 30 min: 7–9% CPU, 2.7 GB flat.
- Follow-ups: `diff_sync` / `diff_targets` call `git_command()` directly, bypassing the gate and the spawn-rate
  counter, and lead 95's startup profile. Pre-install engine soak (gate 3) not built yet.

## In flight (snapshot 2026-11-16 15:45; coordinator da545eee)

- **Installed: release 95** (integration dee3aa810 + 5d6b99faf): git spawn storm and sidebar PR-row scans fixed.
  Mikey restarted; typing is responsive. perf-sample at 3 min: 6–15% CPU, 2.6 GB (94: 200–380%).
  Post-mortem: `docs/internal/postmortems/2026-11-16-slow-mast.md`; prevention items P1–P7 there.
- Integration is ahead of installed by fd5675865 (network git ops skip the spawn gate); ships with the next release.
- Not a leak: the "8 GB RSS" on 94 is mapped GPU memory (IOAccelerator); 95's physical footprint is 1.1 GB
  (peak 1.8) while RSS reads 6.4 GB. Gate on footprint, not RSS. Still open: `pr_ticket_cache` gh calls and `diff_sync::read_head` lead
  the startup profile; spinner ticks re-render the sidebar (2694931e); freeze watchdog (17c9d278).
- Paused workers unchanged from the previous snapshot (code-ref, board, inline diagrams).

## In flight (snapshot 2026-11-16 15:20; coordinator 443e035b handing off)

- **Installed: release 94; Mikey restarted on it and it still feels slow.** Top priority: keep searching for the cause.
  - Already fixed: 93 bounds and caches diff-sync watch-budget walks (they pinned ~8 cores after every restart). 94
    stops the menu-bar item redrawing every ~900 ms (≈40% of the main thread was in NSStatusItem _updateReplicants;
    it's gone in the 94 sample).
  - Latest sample (94, 43 s after launch, ~225% CPU, 2.6 GB): the hottest Mast frames are `mast_engine::repos` (768),
    `source_control` (159), `rpc::handle_request` (154) and `repos::disposable_worker` (108). That is git fan-out:
    `git branch -r --contains …` and refs/branches across ~110 Cofactr worktrees (ui 62, flagship 44, root 12). Mikey is
    removing old Cofactr worktrees with a Claude Code session in a terminal.
  - Earlier: the main thread spent ~¾ of its time in Overview (sidebar) prepaint, which is perf follow-up 2694931e (spinner
    ticks invalidating ancestors, unchanged sidebar rows redrawn).
  - Method: find the Mast pid with `ps -Ao pid,command | grep MacOS/mast` (skip ` mcp`), run `sample <pid> 3 -file …`,
    and count `mast_*` frames; look at the main thread separately. The log is `~/.zeron/logs/mast-headed.log`.
  - Next steps: profile the repos/source_control git fan-out (who calls it, how often, per worktree?), then cap its
    concurrency and cache per worktree with event-driven refresh. Then the sidebar prepaint fix. Add the freeze watchdog
    (17c9d278) so freezes leave a stack.
- **Build discipline:** at most 2 builds at once, counting the coordinator's own. No builds are running now.
- **Paused workers (no builds until Mast is fast):**
  - Code reference + Add to chat (chat bfbe2328, branch `feature/code-ref-add-to-chat` at d12ff36b7): done except the
    final engine, UI and RPC test runs on its head. When resumed, it should run them, then raise Ready to merge →
    code-review-nav.
  - Board (chat a0c94ee7, branch `feature/coordination-board`): it has the review-fix list (perf, real table, labels,
    buttons). It was interrupted mid-fix; its target/ was deleted.
  - Inline diagrams (chat 43132a65, branch `feature/inline-diagrams`): it has the review-fix list (default off, don't
    replace the pane, LRU and concurrency caps, observer outside render). It was interrupted; its target/ was deleted.
  - Word diff and review phase 5 are not started.
- **Needs-you:** the approval rule for offscreen ui-preview renders (78abd9dc); the plan's two calls (how editor-like
  Mast gets; posting to GitHub on Submit); the older items listed in the previous snapshot.

## In flight (snapshot 2026-11-16 afternoon; coordinator 443e035b)

- **Installed: release 92** (91 plus the composer crash fix). Mast crashed on 90 and restarted onto 91; it needs another
  restart for 92. Mast was laggy with 4 Rust builds running (load 44 on 18 cores). The main thread is mostly Overview
  (sidebar) prepaint, which is the perf follow-up 2694931e; the coordinator proposed starting it after wave 1.
- **Code review, navigation and coordination** (item bb3a9716, plan mast-artifact:060317a6, mockups 9229f124 and
  b860e121): wave 1 builds on branch `code-review-nav`, cut from release 91, so it can be rolled back as one unit. Workers
  branch from it and merge into it, not into work-outline-implementation. Wave-1 items (briefs in their descriptions):
  - code reference + Add to chat (187334ff)
  - word diff + expand context (a33d34b9)
  - board (d72281c0)
  - review phase 5 (c5627f01)
  - inline diagrams (9c17adbe)

  Run order: ref + board, then word diff + diagrams, then phase 5 alone. Open calls for Mikey: how editor-like Mast
  gets, and whether Mast may post to GitHub on Submit.
- **Declined:** coordinator context meter and hand-off (Mikey watches the chat's context indicator). Sender provenance
  held until needed.
- **Skills:** talk-to-me gained "Yes/no questions" and "Answers he can read in one pass" (backup
  ~/skills-backups/2026-11-16-answer-shape/).
- **Aside, not decided:** Mikey asked whether Mast should be project-scoped like VS Code. The coordinator suggested a
  window per Space with a global Needs-you, to be tested in the pop-out-window spike.

## In flight (snapshot 2026-11-16; coordinator 9fd580ac handing off)

- **Installed: release 90.** Releases 86–90 under this coordinator: Needs-you cards in place + smaller outline reads (86),
  worker Changes in one click + GitHub-style file tree (87), workflow pack + agent logging/error fixes (88),
  coordinator reach + approach options (89), review follow-ups (90). Confirm Mikey has restarted onto 90, then
  verify the decision-sheet table wrapping: artifact_render_check on docs/artifact-kit/decision-sheet-example.json,
  800 px wide, light and dark. The Data model rejected-table card must not overflow.
- **Running:** nothing.
- **Waiting on Mikey (Needs-you):**
  - approval rule for Codex cargo test in worktrees (ec30a2df)
  - three proposed Space-root instructions: coordinator talk rules (19300dcc), don't poll finished sessions
    (5d1d1f92), shell/CI habits (ac32201c)
  - pinned tabs A (eb86590b; recommend approving all three)
  - older: a53732a1, 99cf645c, 1ad5d3ee
- **Not started, needs Mikey's go:** coordinator context-size meter plus hand-off past ~200k tokens (the real Opus
  cost driver per the quiet-wakes research); freeze watchdog (17c9d278); perf follow-ups (2694931e); structured
  sender provenance, to re-enable last-sender reach safely.
- **Cofactr (other Space, Mikey pastes):**
  - The Space-root instructions (coordinator talk rules, shell/CI habits).
  - An admin-chat prompt to clean stale `cofactr/workspace/.worktrees` (145 GB, 135 worktrees): dry-run table,
    approval, then `git worktree remove` only.
- **Disk:** 178 GB free after `cargo sweep --time 3` (69 GB), the nvm cache plus old Node versions (12 GB) and a Docker
  prune. A Thunderbolt 5 external SSD for build caches is on hold: prices doubled in 2026.
- **Skills:** the friction edits are live in ~/.agents/skills (backup ~/skills-backups/2026-11-12-friction/). On
  2026-11-14 project-planner learned approach-row `options` (backup ~/skills-backups/2026-11-14-approach-options/).
- **Lessons:**
  - Terra stops early when a design choice or the finish line is vague. Give it a numbered checklist ending in the
    Required Action; when it says "blocked", ask exactly what and answer with decisions. Use Sonnet if it stalls 3+
    times.
  - Review every permission-scope change adversarially. Coordinator reach first shipped with workers able to
    approve each other's prompts, and the review caught it.
  - Worker reports use STATUS / RESULT / NEXT. Worker updates that need nothing from Mikey get one line, or none.

## In flight (snapshot 2026-11-12 evening; coordinator 9fd580ac)

- **Installed: release 90** (89 and 90 built overnight 11-14 while Mikey was away; he still needs to restart).
- **Next:** approval rule proposal for Codex cargo test in worktrees (ec30a2df); three proposed Space-root
  instructions (talk rules, no polling finished sessions, shell/CI habits) await Mikey.
- **Terra pattern:** ends turns early when a design choice or the finish line is vague; workflow pack moved to Sonnet
  after 6 stops. Give checklists that end in the Required Action, and answer "what exactly blocks you" with decisions.
- **Orchestrator friction** (report mast-artifact:1e838a8e, item 62aa8fe0): Mikey picked all four fixes.
  - Skill edits are live in `~/.agents/skills` (implement-ticket, project-planner, talk-to-me); backup in
    `~/skills-backups/2026-11-12-friction/`; awaiting his keep/undo (request 289320e7). He still has to paste the
    Cofactr Space instruction and tell the kit coordinator (now 766c6a8e) to re-read the skills.
  - Cards in place shipped in 86.
  - Quiet wakes research (branch `orchestrator-quiet-wakes`, `docs/internal/plans/quiet-wakes.md`): holding routine
    wakes saves ~4% of cache reads, so it's shelved, along with Jev stall triage. Coordinator context growth
    (110k → 566k per call) drives the Opus cost. Proposed next: a coordinator size meter plus hand-off past ~200k;
    awaiting Mikey. Coordinator reach and answer routing (D/E) are still wanted for friction.
- **Running:** workflow pack (Terra 2bbe25e5) as before.
- **Waiting on Mikey:** pinned tabs A (eb86590b); skill edits (289320e7); older a53732a1, 204f55c6, 99cf645c, 1ad5d3ee.

## In flight (snapshot 2026-11-12; coordinator f89abb61 handing off)

- **Installed: release 85.** Releases 78–85 this session: sheet note keys + Accept/Answer differently/Don't do this
  (78), hand off picks harness/model (79, full suite), render check fixed (80, 81), structured `plan` artifact kind
  (82), artifact Export HTML/PDF/markdown (83), visual audit fixes (84, full suite), PR #1 idle CPU (85).
- **Running:**
  - Workflow pack, Terra chat 2bbe25e5, branch `workflow-pack`. It ends its turn after a commit or two, so nudge it
    with "continue until Ready to merge". Left: apply item creation + undo, MCP tools, confirmed apply, Settings UI.
  - Orchestrator-friction audit, Sonnet chat c8dce614, item 62aa8fe0: why planning with orchestrators is frustrating,
    using the Cofactr kit redesign chats (coordinator 532a490a), plus the phase 0 go/no-go (9516dd7c). Raises
    "Review: orchestrator friction".
- **Waiting on Mikey:** pinned tabs A revised (mast-artifact:18746fbb, request eb86590b: overflow rule, initials tiles in
  the Space colour derived from the id, Pinned group in the sidebar); older: active-checkout design (a53732a1),
  agent setup audit (204f55c6), iOS research (99cf645c), Horizon worker idle (1ad5d3ee).
- **Next queued:** build pinned tabs once decided; approach questions with 2–3 options (7db226ae); freeze watchdog
  (17c9d278); perf follow-ups (2694931e, partly covered by PR #1); render-check overlap heuristic flags wrapped
  inline `code` chips (noise); verify release 84's sheet fixes with artifact_render_check on
  docs/artifact-kit/decision-sheet-example.json after Mikey restarts.
- **Mikey still has to paste into the Cofactr kit chat 532a490a** (other Space, unreachable): re-read
  project-planner, republish the plan as `kind: "plan"`, mark droppable sheet rows `optional: true`.
- **Rules learned this session:**
  - `main` is fast-forwarded to the integration head in both repos after every push.
  - The Cofactr palette is for Mast branding assets only; UI styles (Default, Industrial, Horizon) are deliberate and
    judged on their own intent. The Area taste (one surface, no lines) applies to Area only.
  - Review worker visuals yourself before merging: `artifact_render_check` (artifacts; also JSON by path since 84)
    and `scripts/ui-preview.sh` (Mast screens, offscreen PNGs; `handoff` screen exists). UI workers attach a sheet.
  - Disk: orphaned `*.rcgu.o` (`find <target> -name '*.rcgu.o' -links 1 -mmin +15 -delete`) freed ~65 GB at once.
  - Admin grants are per Space; Cofactr chats need a read-only snapshot (~/mast-metrics, .worktrees/phase0-data).

## In flight (snapshot 2026-11-11 evening; coordinator f89abb61)

- **Installed: release 85** (PR #1; touched-crate tests on top of full-suite release 84).
- **Auditing:** orchestrator friction in the Cofactr kit redesign chats + phase 0 go/no-go (item 62aa8fe0, Sonnet chat
  c8dce614; read-only snapshot under ~/mast-metrics/snapshots/2026-11-12-friction).
- **Visual audit** 2026-11-12 (mast-artifact:d3e9de4d rev 3): 7 bug fixes shipped in release 84. Style colours of
  Horizon/Industrial are deliberate, not findings; the Cofactr palette is for branding only. Pinned tabs A revised
  (mast-artifact:18746fbb, request eb86590b) awaits Mikey's three decisions.
- **Plans are structured now:** the kit plan published as kind `plan` renders clean in the app (test artifact
  5381891f on the Mast Dev item). The project-planner skill (~/.agents/skills, backup
  ~/zeron-dev-app-backups/skills-2026-11-11-project-planner-v2) now publishes plan JSON per
  docs/artifact-kit/plan-schema.md and copies each ticket's generated Linear body. Follow-up: the overlap
  heuristic flags wrapped inline `code` chips (noise). Workflow pack (2bbe25e5) still in progress.
- **PR #1 approved** on GitHub at c6bf195c1 (Mikey: approve if good) with two non-blocking follow-ups; not merged.
- **PR #1 (David Wallach, idle CPU + input latency):** reviewed (item 953b96ee, Required Action 08002e19):
  merge after three fixes (nested-submodule diff filter, scheduled-run backfill marker, @-completion freshness on
  over-budget checkouts). David force-pushed c6bf195c1 (dropped the scheduled-task commit, so the backfill finding is gone); two
  findings remain. Waiting on Mikey: David fixes vs we fix. Review checkout `.worktrees/zeron/mast-pr-1`.
- **main is kept current (Mikey, 2026-11-11):** after each integration push the coordinator fast-forwards `main` in
  both repos (`git push origin origin/work-outline-implementation:refs/heads/main`; parent `zeron-work-outline`).
  Fast-forward only. Workers never touch main.
- **Resumed:** Terra chat 2bbe25e5 (workflow-pack export/import, branch `workflow-pack`).
- **Next queued:** approach-question options (7db226ae); global pinned-chat tabs strip (design
  mast-artifact:6b34d7fb, Mikey picked A, global); freeze watchdog (17c9d278), render-probe bug (bb6e6f10), perf
  follow-ups (2694931e); Phase 0 go/no-go (9516dd7c) was due 11-12.

## In flight (snapshot 2026-11-11 late; coordinator 844aab58 handing off)

- **Installed: release 77** (fast-tracked; targeted tests only). It has the decision sheet Send fix (be62a4719), row
  ids, batched decisions, the queue silent-drop fix, the usage-only menu bar, and smaller dev builds (7c53f6b54).
  The next normal release must run the full suite.
- **Running, URGENT (Mikey is blocked):** Terra chat 921177a0, branch `sheet-note-keys`, item e6f833a5.
  - Fixes the decision-sheet note field dropping e/n/p/t/v (Mast shortcuts swallow keys while a web input is
    focused).
  - Replaces Reject with **Accept / Answer differently / Don't do this**. "Don't do this" shows only on
    `optional: true` rows.
  - When it pushes: review, merge, then fast release 78 with the pattern in /tmp/release77-run.sh (targeted tests +
    installed build, detached via python setsid), install, and tell Mikey to restart.
- **Paused:** Terra chat 2bbe25e5 (workflow-pack export/import, branch `workflow-pack`). Tell it to resume cargo
  after release 78.
- **Next queued:**
  - Approach questions with 2–3 selectable options (7db226ae)
  - Global pinned-chat tabs strip in the main titlebar, across Spaces (Mikey picked A, global; design
    mast-artifact:6b34d7fb; not started)
  - Freeze watchdog (17c9d278), render-probe bug (bb6e6f10), perf follow-ups (2694931e)
  - Phase 0 go/no-go (9516dd7c) was due 11-12.
- **Process rules from today:**
  - At most 2 building workers.
  - New worktrees get `.cargo/config.toml` with incremental off.
  - Workers never delete anything; the coordinator frees disk (main `target/debug/incremental` and
    `target/installed` are safe).
  - Releases run detached.
  - Mikey reviews visuals, so look at worker screenshots before merging.
- **Skills** (one copy each in ~/.agents/skills): project-planner now has scope vs approach questions, an
  `optional` flag, no boilerplate questions, batched decisions via artifact_get_decisions, and ids in chat.
  talk-to-me has the LLM-tics table.

## In flight (snapshot 2026-11-10 17:30; coordinator 844aab58, near its weekly limit)

- **Installed: release 77** (fast-tracked: decision sheet Send fix be62a4719 + row ids; targeted tests only — run a full release suite with the next release).
- **Running:** Terra chat d1ac228d (item 3b9a83cb, branch `decision-sheet-v2`, worktree `.worktrees/zeron/decision-sheet-v2`).
  - Mikey picked the decision-sheet redesign "B + C": B's "Needs you" list with Accept/Reject on top, C's tinted
    flow chips below.
  - Mockups and real data are in /private/tmp/sheet-v2/; the design artifact is mast-artifact:978a4499.
  - Scope: optional child status (inherits), a `recommendation`/`question` field, top-level-only counts, the B+C
    renderer and kit CSS, and row Accept/Reject/comment through the bridge, stored per artifact, read back via
    artifact_get_decisions, with ONE aggregate Needs-you per sheet.
  - When it lands: look at its 1200/520 light/dark screenshots, merge, release 75 using the detached runner
    (/tmp/release74-run.sh pattern), then delete the worktree's target/.
- **Next on the docket (Mikey hasn't said go):** freeze watchdog (17c9d278), render-probe bug (bb6e6f10), queue
  race (79760f82), perf follow-ups (2694931e). Phase 0 go/no-go 9516dd7c is due 11-12.
- **Disk:** about 113 GB free after clearing target/debug/incremental again; it refills quickly, so clear it
  before releases.
- **Skills** (all one copy each in ~/.agents/skills, linked into claude/codex/cursor): project-planner
  (decision-sheet JSON, rows = changes only, top-level decisions, ~10–25 open, no recommendations in titles),
  implement-ticket (orchestrated mode; workers read the sheet and never publish it), review-pr, code-review,
  talk-to-me (the "LLM Writing Tics: never" table applies to all writing including UI and design). The Cofactr
  Space-root instructions already carry review-pr + the quiet-orchestrator rule.

## In flight (snapshot 2026-11-10 13:00; coordinator 844aab58)

- **Installed: release 74** (restart needed). Coworker testing handed off by Mikey 2026-11-10. No workers are running; the Horizon worker 8ac73b69 is
  idle.
- **Coworker hand-off:** ready on the code side. Coworkers build from source off the public, standalone repo; there's
  no DMG and no workflow pack. Waiting on Mikey to pick the first coworker for a clean-Mac run. The iOS companion
  stays undocumented in the README until DevOps approves it.
- **Process redesign (from the Kit Scheduler session a2e6b013, Cofactr Space):**
  - Diagnosis: three gates per ticket in `implement-ticket`, about 70 repeated "Waiting on you" lists, decisions
    buried in a plan that reached rev 44, and UI seen only at local QA.
  - Proposed: one gate per ticket (workers push and open draft PRs; Mikey reviews and merges), a quiet orchestrator,
    a **decision sheet** (contracts and constraints plus flows, mast-artifact:1af1d319), and a saved local-test
    recipe.
  - Mikey wants an **artifact component library**. Recommended: A, a built-in CSS kit injected by the fragment
    host, then B, data-driven kinds like `decision-sheet` JSON → kit HTML. Not C, native gpui. Awaiting his go.
  - Skill edits (`~/.claude/skills/project-planner`, `implement-ticket`) need his OK.
- **Parked today:** upstream Zeron port triage (07486a45), chat fork/rollback (729bb9ad, design mast-artifact:7b47bc3d).
- **Repo:** detached from the fork network; public; credits and cleanup merged. Personal values live in parent
  `docs/private/`.

## In flight (snapshot 2026-11-08 21:15; coordinator 844aab58 handing off)

- **Installed Mast: still release 70.** No Mast-side workers are running. The Horizon worker (8ac73b69) waits for Mikey's
  next batch.
- **iOS companion works end to end on Mikey's iPhone 13 mini.** Send, read and answer are confirmed live; Stop is not
  yet tested (the button only shows while the viewed chat is working). Merged tonight, so integration is at c5589eae:
  - 7874665e: writes wait for `.active` after Face ID
  - c5589eae: **Face ID only on unlock** (launch and return from background), no longer per action. Mikey asked for
    this; writes need an app unlocked by real device auth, and the no-passcode bypass is read-only.

  Both are reinstalled on the phone. The worker (Terra chat 3c892a57) is idle with no open requests, and all four
  iOS QA requests are resolved.
- **Bridge:** Mikey runs it from `.worktrees/zeron/ios-companion/apps/ios/bridge`. The main checkout has no `dist/`;
  use the worktree for CLI commands. agent-mode-tools was found at `shared:answer`, probably because Mikey pasted
  the share line, and was reset to `send`. Paired phone: **da178287**. Stale phone pairings d7ae9871 and 47615a59
  can be revoked; Mikey hasn't answered yet.
- **Security docs updated (revision 2 of each):**
  - Security brief mast-artifact:0ce919c7: current state, Face ID on unlock as a trade-off, ACL open. It says the
    app already runs on the managed phone; Mikey should OK that wording before sending.
  - Review mast-artifact:cab6f4c5: status and commit for each finding.

  Sources are in `/private/tmp/mast-ios-security/`, with the previous versions kept beside them.
- **Next options (unchosen):** desktop Settings → Phone page (Rust + release); auto-start the bridge; photo
  attachments (9719a85d).
- **Waiting on Mikey:** personal-workspace revamp (204f55c6), coworker readiness (2fda739e), iOS research review
  (99cf645c), and whether to revoke the stale pairings.

Real values: agent-mode-tools docs/private/ios-device.md.

## In flight (snapshot 2026-11-08 20:00; coordinator dd899921 handing off)

- **Installed Mast: release 70** (93c138ba). No Mast-side workers are running. The Horizon worker (8ac73b69) waits for
  Mikey's next batch; its queue so far is just the Horizon Inbox panel divider line.
- **iOS companion on Mikey's real iPhone 13 mini** (UDID <iphone-udid>, now trusted, so the work phone's MDM allowed
  the dev cert). Integration has 881e60cc; branch `ios-companion` is in worktree `.worktrees/zeron/ios-companion`.
  Worker: Terra chat 3c892a57, item 14c99c51.
  - **In progress:** fixing a send bug. `authenticateWrite()` (Companion.swift ~351) checks
    `applicationState == .active` immediately after Face ID, but the app is `.inactive` while the auth sheet
    dismisses, so every send/stop/answer fails with "Authentication or device credential unavailable".
  - When it's pushed: merge `origin/ios-companion` (check the diff stays inside `apps/ios/` + the doc), then reinstall
    on the phone yourself:
    `xcodebuild -project apps/ios/Mast.xcodeproj -scheme Mast -destination 'id=<iphone-udid>'
    -derivedDataPath /tmp/mast-ios-device -allowProvisioningUpdates DEVELOPMENT_TEAM=<team-id>
    CODE_SIGN_STYLE=Automatic PRODUCT_BUNDLE_IDENTIFIER=computer.mikey.mast.companion.dev build`, then
    `xcrun devicectl device install app --device <iphone-udid>
    /tmp/mast-ios-device/Build/Products/Debug-iphoneos/Mast.app`. Free team: the build expires after 7 days.
  - **Bridge** (Node **22**: `export PATH="$HOME/.nvm/versions/node/v22.17.0/bin:$PATH"`, in
    `apps/ios/bridge`). Mikey runs `node dist/cli.js start --tailscale-user <tailscale-login> --url
    https://<tailnet-host>` in his terminal; add `--pair` only to pair. After bridge changes, run
    `npm run build` and ask him to restart it. Sharing persists: agent-mode-tools is `shared:send`. Don't re-run
    `spaces share … --allow answer`. Paired devices: phone d7ae9871, simulator f55fed2a. CLI: `spaces`, `devices`,
    `revoke <id>`, `audit N`.
  - Tailscale: Mac <tailscale-ip> and phone <tailscale-ip> on tailnet <tailnet-host> (<tailscale-login>). Serve is
    tailnet-only, Funnel off, HTTPS certs on.
  - **Security:** the Opus review (mast-artifact:cab6f4c5) found 0 Critical, 1 High, 5 Medium, 13 Low; all fixed and
    merged (0f615ea2). Security brief: mast-artifact:0ce919c7. Its "fixes planned" wording is now out of date: update it
    to "fixed" before Mikey sends it. The review is pending.
  - **Done this session:** artifact viewer, three styles + Match Mac, live updates, Stop, QR pairing, loading states,
    44 pt tap targets, capability-aware controls and errors.
  - **Open items:** photo attachments (9719a85d, needs a Rust attach path); desktop Settings → Phone page
    (Rust + release); auto-start the bridge; the xcodebuild approval rule (e866c323) awaits Mikey.
- **Waiting on Mikey:**
  - personal-workspace `chore/agent-setup-revamp` (204f55c6): merge, and the Space vs AGENTS.md line question
  - coworker readiness (2fda739e)
  - active-checkout design (a53732a1); "Phone QA test (choose)" (4acead10) can be closed
  - iOS QA requests: refinement 3 (52cb2b03), artifacts (6966a1cc), security fixes (663aa009), spaces+lock (b36e5074).
    He has used most of these live, so resolve them as each is confirmed.

## In flight (snapshot 2026-11-08 17:30; coordinator dd899921)

- **iOS companion merged on integration** (a1ac86d6; `apps/ios` + `docs/internal/ios-companion.md` only, no Rust).
  SwiftUI iOS 17 app + Node 18 bridge (loopback 127.0.0.1:27655 → engine IPC 27654, unscoped `mast mcp` for chat
  tools). Shipped:
  - Needs-you inbox, and reply & resolve with desktop parity (live-QA'd: audit shows answer sent + resolved)
  - chats with tail paging, system/relay rows, worker chats hidden
  - send, Stop, and SSE live updates
  - per-Space sharing, default deny (`node dist/cli.js spaces share agent-mode-tools`)
  - Face ID/passcode lock and app-switcher cover
  - confirmations unless Quick actions; metadata-only audit log; per-device revoke; QR pairing
  - Default/Industrial/Horizon styles with Match Mac (read-only `GET /appearance`), fonts EB Garamond + Geist (OFL)

  Mikey QAs in the Simulator (iPhone 15 Pro; workers test on the Pro Max E50FC8FD). Tailscale is set up on the Mac
  (tailnet <tailnet-host> `tailscale serve --bg http://127.0.0.1:27655` is tailnet-only, Funnel off). His work iPhone
  13 mini (the work phone's MDM) **blocks dev-signed apps**, so phone use awaits approval. The signed build (Personal
  Team <team-id>, bundle `computer.mikey.mast.companion.dev`) is installed there via devicectl but can't open.
  Open items: security pass before review (84df7cc6), photo attachments design (9719a85d; needs a Rust attach
  path), desktop Settings → Phone page (not started; Rust + release). Proposed approval rule for iOS xcodebuild
  (e866c323) awaits Mikey.

## In flight (snapshot 2026-11-08 01:31; coordinator dd899921, overnight)

- **Installed: release 70** (93c138ba, 01:31; nextest 8/8, UI 2601/0/13). Backup of release 69:
  `~/zeron-dev-app-backups/mast-20261108-0131-pre-release70`. Mikey has not restarted yet. It contains:
  - `horizon-fixes-1`: Horizon SEND and primary chrome use the theme accent with saturation capped at 0.22, so they
    stay muted on any theme; Design-panel overrides win. Horizon's selected Appearance/Style/Layout cards get a soft
    accent wash and a check badge.
  - `migrate-linear-team`: the **last screen migration slice**. The Linear panel, project picker, settings, Inbox host
    and Team UI are on shared components, with offline previews (`linear`, `linear-settings`, `team`). Ratchet
    442/227/3/2 → 410/205/3/2; Default+Minimal compare 1.0 / 1.0 / 0.9999996. Follow-up: the Horizon Inbox panel
    still draws a divider rule.
- **iOS companion** (item 14c99c51, Terra chat 3c892a57, branch `ios-companion` at 94255e81, **not merged; Ready to
  QA** request 5fe60698; iOS unit 4/4, bridge 8/8; live engine/phone/Tailscale unverified): native SwiftUI app
  (bundle `computer.mikey.mast.companion`, iOS 17 target) plus a Node bridge under `apps/ios/`. It must not change any
  Rust/Cargo files. The bridge binds to loopback, connects to the engine IPC on 127.0.0.1:27654, and exposes only
  allowlisted HTTP endpoints. Pairing uses a one-time code and a Keychain token; Tailscale Serve is the phone path.
  v1 scope: a read-only Needs-you inbox, running chats, and send message. Reply & resolve stays off until desktop
  parity is verified. Research: mast-artifact:05bc2b8a. Coordinator decisions (Mikey delegated any choice that doesn't
  affect the Rust build): native now, Node bridge, iOS 17, no APNs, no approvals in v1. QA steps:
  `docs/internal/ios-companion.md` on the branch.
- **Standing Horizon worker** (8ac73b69): waiting until 2026-11-08 12:00 for Mikey's next batch.

## In flight (snapshot 2026-11-07 23:35; coordinator dd899921)

- **Installed and current: release 69** (c118d5cf, 23:35; nextest 35/35, UI 2599/0/13). Backup of release 68:
  `~/zeron-dev-app-backups/mast-20261107-2335-pre-release69`. Two changes:
  - **Area is now Horizon** (`rename-area-to-horizon`): `UiStyleId::Horizon`, the `horizon-light`/`horizon-dark`
    themes, the design-panel key `horizon`, the gallery "Horizon outline tree", `docs/internal/styles/horizon.md`.
    Legacy values still load: `UiStyleId` deserialises `"area"`, the theme registry maps `area-*` → `horizon-*`, and
    style overrides accept `area` (`horizon` wins if both exist). Area/area.tech palette attribution is kept.
  - Horizon list discs use the text foreground (`area-list-bullet-color`), so they follow the agent colour in replies.
- The standing Horizon worker (chat 8ac73b69, item ab2dc8d0, still titled "Area style tweaks") is idle and waiting
  for Mikey's next batch. Stale origin branches still exist: `archive/ios` (keep) and `audit-*` (coworker readiness
  2fda739e decides).
- **Personal workspace:** `chore/agent-setup-revamp` was pushed for Mikey's review (request 204f55c6). Open question:
  make personal-workspace a Mast Space, since its new AGENTS.md points tracking at the Work Outline, or soften that
  line.

## In flight (snapshot 2026-11-07 23:10; coordinator dd899921)

- **Installed and current: release 68** (9437ef38, 23:10; nextest 8/8 for the touched crates, UI 2597/0/13). Backup of
  release 67: `~/zeron-dev-app-backups/mast-20261107-2310-pre-release68`. Area tweaks from Mikey, all built by Terra:
  - `area-tweaks-1`: Area replies wrap in a 720 px reading column. A flex wrapper had caused intrinsic-width
    overflow. The first-block identity dot is gone, and lists use small discs with a hanging indent.
  - `area-tweaks-2`: agent colour follows the provider, not a chat-id hash (`agent_identity_color`). Claude is ANSI
    amber, Codex green, other harnesses blue, an unknown model slate. The danger-hue guard falls back to slate (Claude
    on Mast Light). Applies to replies, composer, sidebar chat rows and harness tabs. Transcripts store no per-message
    model, so replies follow the chat's current config.
  - `area-outline-tree`: Mikey picked option A, rounded nesting. Area's outline draws soft rounded containers per depth,
    with tint capped at two levels and a neutral selection tint. The gallery story "Area outline tree" keeps A/B/C.
  - `area-tweaks-3`: shared style-aware composer menus cover model/harness, reasoning/options, branch/project pickers,
    file mentions and slash/skill completion. Area is one surface with a soft shadow and pill highlights; Industrial
    uses its panel sizing; Default is pixel-identical. This was the "menus" slice.
  - Gallery inspector **Copy details** (`inspector-copy-details`, gallery-only): copies source file:line, id path,
    bounds, the set style fields as hex, and the global UI style/appearance. It cannot see the gallery story.
- **Queued on the standing Area worker** (chat 8ac73b69, item ab2dc8d0): batch 4 makes Area list discs follow the
  agent colour. Batch 5 is `rename-area-to-horizon`: Mikey chose **Horizon** ("aspirational, broad, futuristic,
  optimistic"), item db9d6b22, with serde aliases or migration so saved `area*` values keep loading.
- **Personal workspace agent setup** (item ded4ca6d, Sonnet chat 6ad9a893; repo `~/Projects/personal-workspace`, not
  a Mast Space): proposal mast-artifact:7d7cc861 rev 7 is answered by Mikey. Implementation goes on branch
  `chore/agent-setup-revamp`. The worker waits for Mikey's own "go" in its chat; it won't take the coordinator's.
- **iOS**: upstream's SwiftUI phone client is on `origin/archive/ios`. It needs the removed edge relay. Mikey asked what
  it was; any revival would be research-first and Needs-you-centred. Don't delete `archive/ios` without asking.

## In flight (snapshot 2026-11-07 17:20; coordinator e81de680)

- **Installed and current: release 67** (b9d61506, 17:17; nextest 2485/2485, UI 2590/0/13). Backup of release 66:
  `~/zeron-dev-app-backups/mast-20261107-1717-pre-release67`. Design-iteration tooling (research:
  mast-artifact:2b0aa925…, item 7b93fcb1), Mikey's rule: tweak/prototype in real components, not HTML mockups.
  - `design-gallery`: per-view styles (`Theme::with_ui_style`), story registry with states, Board prototype story,
    `./scripts/gallery-watch.sh` (Mikey runs it; rebuild-on-save, keeps selection). No release needed to use it.
  - `design-panel`: `mast.style-overrides/v1` file in the profile, watched live; Settings → Developer → Enable Design
    (off by default) with component-backed roles (radius, sizes, per-surface border/page colour, primary/secondary
    colour role, tracked caps, heading font) across toolbars, sidebar, header chips, composer/SEND,
    message/tool/code panels, outline rows. Save writes the file (gallery follows it); Copy patch → coordinator turns it
    into permanent defaults. No-file renders 18/18 pixel-identical. Docs: docs/internal/design-panel.md.
  - `design-render`: warm preview server `scripts/ui-preview.sh --serve <dir>` (request.json/response.json, manifest),
    `.claude/skills/mast-ui-render/SKILL.md`, approval matcher accepts `NAME=$PWD[/rel]` env prefixes only inside a
    rule's cwdScope (canonical path check; everything else still fails closed; reviewed by the coordinator),
    `dev-inspector` feature (⌘⌥⇧I read-only inspector; NOT for the installed app: always-on per-element cost),
    hot-patch spike: Subsecond not viable (tip-crate restriction).
- **Merged after release 67 (source-run, no release needed):** `gallery-screens` (12e34d5a): gallery-only inspector
  (⌘⌥⇧I; the installed app has no inspector feature) and a Screens section built from the preview fixtures (chat +
  sidebar, details/Needs-you, Overview/outline, populated changes, Appearance, Developer → Design, browser), keys
  1/2/3 style, ⇧1-3 layout, L/D, T theme; coverage doc docs/internal/gallery.md. No workers running.
- **Waiting on Mikey:** proposed approval rule "Headless UI snapshot renders (mast-ui / mast-gallery)" (request
  0302ff3e); coworker readiness findings (2fda739e: ad-hoc DMG vs notarised, fresh-Mac tester, stale branches);
  active-checkout design (a53732a1); UI-styles question D Board (now a gallery prototype; list|board toggle later) and
  H (style per person vs per machine). Workflow pack parked (item 2bdaeb10). Phase 0 go/no-go 2026-11-12.
- **Workflow for design tweaks (agreed 2026-11-07):** value tweaks → Mikey uses the Design panel and sends "Copy
  patch" → a worker makes them permanent defaults; structural tweaks → standing per-style Terra design worker, Mikey
  watches the gallery; new layouts → gallery story first. Review previews before merging; never use HTML mockups.
- **Follow-ups:** Area SEND under non-Area themes; Area Light selected Style/Layout cards grey; Linear/team and menus
  slices; standing per-style design workers once Mikey sends a tweak list; engine test
  `rpc::tests::local_opening_tail_arrives_before_full_mirror_and_keeps_all_history` fails when the engine lib suite
  runs outside the release harness (timing), passes in release runs.

## In flight (snapshot 2026-11-07 00:10; coordinator e81de680)

- **Installed and current: release 66** (f92924d9, 00:08; UI 2581/0/13). Area details section headings share one
  size and left edge with gutter chevrons; Area list controls go through the shared `toolbar_control`
  (`area_toolbar_control` removed). Backup of release 65: `~/zeron-dev-app-backups/mast-20261107-0008-pre-release66`.
  No workers running; all worktrees removed.
- **Follow-ups (not started):** Area SEND under non-Area themes uses the bright accent instead of the theme's muted
  fill; Area Light marks the selected Style/Layout cards grey not accent; Linear/team and menus slices; the "UI
  preview sheets" approval rule doesn't match workers' `CARGO_TARGET_DIR="$PWD/…" perl alarm … scripts/ui-preview.sh`
  commands (they still ask; likely the `$PWD` expansion fails closed).

## In flight (snapshot 2026-11-06 23:30; coordinator e81de680)

- **Installed and current: release 65** (cf711028, 23:31). Tests: UI 2581/0/13, nextest 28/28. Backup of release 64:
  `~/zeron-dev-app-backups/mast-20261106-2331-pre-release65`. Mikey must restart to see it.
  - Migrations: `migrate-settings` (settings_parts), `migrate-browser` (browser_parts, browser preview screen),
    `migrate-changes` (changes_parts, `MAST_SNAPSHOT_POPULATED_CHANGES=1` gives the walkthrough fixture 3 files + diffs).
  - `industrial-composer`: composer footer (Model chip sizes to content, captions over their controls, one key
    treatment, muted meter), one frame per reply, shared `toolbar_control` / `toolbar_search_frame` in
    components/mod.rs at the layout control height (Overview, outline/details, chat header, titlebar; an `overview`
    preview screen now exists). `changes-industrial-radius`: Changes header keys + square notice.
  - `area-quiet-chrome` parts 1–2 (Mikey: Area should be "all white, no dividing lines", like area.tech
    Character_Tag_Demo): one surface, no pane rules, quiet sidebar/Needs-you/header chips, inline tool groups and light
    code panels, Area arm of toolbar_control (borderless muted, one filled primary), quiet outline (no guides, fixed
    disclosure slot, selected pill, muted counts, hover chevrons) and details, slate SEND.
  - `style-themes`: "Recommended for <Style>" + opt-in "Use <Style> colours" (never auto-switches); Area Light/Dark
    sampled from the reference (#FFFFFF, #0BA7FF, #FFB800, #FF7759, #B388E4, #1DCB43, slate #8EA4B8); green/red diffs.
  - Ratchet 442/227/3/2.
- **Running:** Area v2 worker (chat 2ce22eab, branch area-quiet-chrome): details section headings (collapsible ones
  are bigger and indented ~8px vs static ones), small toolbar-wrapper cleanup. Next release.
- **Follow-ups:** Area Light marks the selected Style/Layout cards grey, not accent; hover-only chevrons not visually
  verified; Linear/team and menus slices not migrated; shell audit ledger + conversation tool/markdown children
  untriaged; Compact light chat snapshot stale.
- Mikey accepted the "UI preview sheets" approval rule (scripts/ui-preview.sh only); snapshot `cargo test` renders
  still ask.

## In flight (earlier snapshot 2026-11-06 night; coordinator e81de680)

- **Installed and current: release 64** (f44b65c5, 19:09): the first three component MIGRATIONS on top of the
  release-63 foundation. Release tests passed (UI 2580/0/13). Backup of release 63:
  `~/zeron-dev-app-backups/mast-20261106-1909-pre-migrations`. Mikey needs to restart to see it.
  - `migrate-conversation` (e335e22e): transcript, composer*, markdown/, composer-facing chips in pickers.rs →
    components/conversation_parts.rs. Area composer/SEND/tab follow the theme accent; SEND sits in the footer row;
    Industrial square send/attach/code panels; MODEL caption no longer clipped. Default+Minimal 1.000.
  - `migrate-shell` (f476d58e): shell.rs, shell/, rail.rs → components/shell_parts.rs. Area sidebar row keeps
    status/queued/failed/archive/jump/PR; Industrial square titlebar keys and rows. Compact snapshots were stale and
    were regenerated (Compact dark from the merged head, Compact light from the foundation).
  - `migrate-outline` (f44b65c5): work_outline/, overview*.rs → components/outline_parts.rs. Default+Minimal
    details 1.000.
  - Ratchet after all three: 486/322/7/5 (was 772/446/29/14).
- **Follow-ups from the coordinator's preview review** (fold into the Industrial/Area polish slice): Industrial
  static detail headings (Chats, Items, Project, Child template) use wider tracking than the collapsible ones and
  wrap the count to a second line ("C H A T S / (2)"); Area date input in details shows a red outline; Area sidebar
  row omits project/branch/remote metadata; regenerate the Compact light chat snapshot from the current head; the
  shell audit ledger and the conversation tool/markdown child elements are untriaged (listed in
  docs/internal/ui-components.md).
- **Next slices (not started):** changes/walkthrough (changes.rs, changes/, files/), settings/appearance,
  browser/artifacts, linear/team, menus/remaining (popover, pickers menu bodies, app_menus, menu_bar). Then Area
  and Industrial polish as component skins.
- **Preview review tips:** previews are 4 themes × 9 style/layout rows; crop cells with `sips -c 455 728
  --cropOffset <row*455> <col*728>`. The rich fixture (`offscreen_snapshot render_style_theme_matrix`) shows code
  and tool panels per style. Comparisons must render the foundation and the branch with the same renderer;
  checked-in snapshots can be stale. A shared CARGO_TARGET_DIR between two worktrees can render the wrong commit:
  check the log says "Compiling mast-ui" from the right path.
- **Disk:** Codex workers' `perl alarm` kills leave orphaned `*.rcgu.o` in `target-clean/debug/deps` (6–11 GB per
  worktree); `find … -name '*.rcgu.o' -links 1 -mmin +15 -delete` is safe mid-run.
- **Waiting on Mikey:** unchanged from the snapshot below (UI preview approval rule 26af0df6, workflow pack draft,
  coworker readiness findings, active-checkout design, enabling suggested approval rules). Hand-off menu for
  worker-less chats: Mikey said leave as is (the agent can still call `hand_off_orchestrator` if the chat has a
  work item). Phase 0 go/no-go on 2026-11-12.

## In flight (older snapshot 2026-11-06 evening; coordinator 1d22a4b9 handing off)

**State at hand-off (read this first):**
- Installed and current: release 63, the component foundation (`ui-components`), which gives Style {Default,
  Industrial, Area} × Layout {Minimal, Compact, Guided} × Theme as three independent pickers. Default+Minimal is
  pixel-identical. Approval rules are installed (release 61). Mikey restarted after release 61; release 63 is
  installed but may not be restarted into yet.
- **Running (Codex Terra), the component MIGRATIONS** (parent item 06a135d5; slice ownership in
  docs/internal/ui-components.md):
  - `migrate-shell` (chat 8d4a5ade, item 8cb5b061): shell.rs, shell/, rail.rs. Titlebar and header buttons done.
  - `migrate-conversation` (chat 54595609, item 90dbc4aa): transcript.rs, composer*, markdown/. Messages, tool
    and code panels, SEND key, Area composer width + 3-line min done.
  - `migrate-outline` (chat 7e43c6b5, item 624586b8): work_outline/, overview*.rs. Rows, status, toolbar, item
    controls, Needs-you buttons done; UI 2577/0.
  Each must: remove all style/layout branches from its files, lower the ratchet baseline (component_lint), keep
  Default+Minimal ≥ 0.999 via the snapshot compare, then run ONE `scripts/ui-preview.sh` and look at it, then raise
  Ready to merge. THE COORDINATOR REVIEWS THE SHEETS ITSELF (Read the PNGs) before merging. They may conflict on
  components/mod.rs; new pieces were asked to go in components/<slice>_parts.rs.
- **Next slices after these merge:** changes/walkthrough (changes.rs, changes/, files/), settings/appearance,
  browser/artifacts, linear/team, menus/remaining (popover, pickers, app_menus, menu_bar). Then Area v2 polish and
  Industrial polish as pure component skins (Mikey: "both inconsistently applied"; Area "all over the place"). The
  Area reference images are the area.tech OpenAI case study (Sidebar, Writer_Demo, Tool_Dock_Demo,
  Character_Tag_Demo, Warp_Code_Demo), described in docs/internal/styles/area.md.
- **Waiting on Mikey (Needs-you):** proposed approval rule "UI preview sheets" (request 26af0df6, one-click accept;
  it stops the per-render approvals); workflow pack draft; coworker readiness findings; active-checkout design.
  Mikey should also enable the suggested approval rules (Space settings → Approval rules).
- **Phase 0 observation** item 9516dd7c waits until 2026-11-12 (waitingUntil set): then re-run the metrics per
  the observation log (mast-artifact:0b48afa2…) and give a go/no-go.
- **Lessons this session:** merging two branches can break the build semantically even when git merges cleanly
  (approval-rules used a helper that ui-components removed); always run at least `cargo build -p mast-ui` right
  after merging, before starting workers from that base. Five concurrent clean builds overload the machine (load
  ~98) and eat about 25 GB each: keep to about 3 builders, prune `target/debug` (`-mmin +180`) after each release,
  delete merged worktrees' target-clean right away. Terra stops after the first slice almost every turn, so send
  an ordered "keep going" list. Workers can't capture the screen, but the coordinator and Mikey can review offscreen
  PNGs; Mikey can also paste screenshots into chat.

## In flight (older snapshot 2026-11-05 evening; coordinator handed off from chat 74bc70e5)

**Coordinator:** a fresh Mast Dev chat created by `hand_off_orchestrator` from 74bc70e5 (which ran 2026-11-03 → 11-05).
Keep it thin: implementation goes to Codex Terra (Mikey's preference to save Anthropic usage). Release runs go
DETACHED (`nohup /tmp/mast-release/releaseN.sh & disown`; the script sets `ulimit -n 4096`); never start one while a
merge is unresolved, and never prune `target/debug` while one is running. Install with
`scripts/install-macos-app.sh target/installed/mast /Applications/Mast.app computer.mikey.mast.dev` (see Install
below). Answer Codex approvals with the real question id (get_chat → pendingInput); the installed build now errors on
unknown ids instead of denying.

**Overnight 2026-11-06 (Mikey asleep; released in batches, installed, Mast NOT restarted):**
- Installed (needs Mikey's restart to take effect):
  - `template-proposals-button`: "Review proposed instructions" shows child-template proposals inline.
  - `overview-toolbar-cleanup`: List mode hidden, with saved List migrated to Outline (legacy List code is still
    there); Overview PR button restored (it vanished in d7a30adde with the retired sidebar); shortcuts button icon
    plus a capped, scrollable popover; Hide stale/Show archived in ⋯; Key removed.
  - `multi-question-ask`: Claude AskUserQuestion accepts typed "Other" answers. Codex approvals stay strict. The
    option-only rejection in friction 8e01dbe1 wasn't reproduced separately.
  - `waiting-until`: `waitingUntil` + reason on items, auto-continue skips them, one due notice (starts nothing),
    MCP mutation, item control and row chip. Once Mikey restarts, set phase 0 item 9516dd7c to wait until 11-12.
  - `coworker-docs`: guides for the walkthrough, Needs-you, hand-off, templates, friction and the menu bar;
    first-run and accounts wording; the DMG hand-off; dev reports moved to docs/internal. Docs only.
  - `ui-styles` (slice 1) + `ui-styles-guided` (slice 2): `uiStyle` setting (Settings → Appearance → Style),
    default Minimal = identity. Dense: scaled type/rows/headers, persistent hover actions, labels, key hints.
    Guided: labels and 32 px controls on selected headers/toolbars/sidebar/composer, status words, filled primary
    actions, dismissible help cards (`dismissedHelpCards`), walkthrough guidance; narrow toolbars truncate labels
    with tooltips. Follow-ups are item a59a87bc: full icon-button audit, true icon-only collapse, titlebar and
    diff-gutter controls. Mikey hasn't seen it yet; revert the two merges if he doesn't want it.
  - `review-this-work` (PR review 4c-1): "Review this work" on worker chats/items (sidebar, Team, item details)
    resolves the local base/ahead range, opens Start Chat in branch-review mode (model/account) and reuses an
    existing review.
  - `review-explain` (4c-2): select diff lines (unified/split, drag or shift-click) or press `e` → Explain; the
    question goes to the matching review chat, else the owning chat (queued if busy), and the reply shows inline
    with Open in chat. "Ask about this step" in the walkthrough. A live agent round trip is untested.
- **2026-11-06 afternoon:** `walkthrough-polish` (walkthrough side text 12/13/11 px, clamped notes), `ui-gallery`
  (`cargo run -p mast-gallery`: isolated fixtures, styles from `UiStyleId::ALL`, keys 1–9 and L/D; Mikey launches it,
  agents never do; sidebar/outline/walkthrough stories are stubs), `looks-industrial` (Industrial look: paired
  light/dark themes with restore, opaque ruled panels, Braun key controls, inset composer, mono counts, #FE4B25
  signal accent; UiStyle grew radius/rule/surface/control-variant/status/font/theme-pair tokens) and
  `spike-ui-snapshots` (macOS offscreen Metal `mast_ui::snapshot::render_to_png`, 24 baselines, image-compare diffs;
  docs/internal/ui-snapshots.md). Snapshot tests need a run outside the Codex sandbox, which means an approval per
  run; the coordinator can run them directly and Read the PNGs to review UI. Mikey disliked Dense/Guided ("not
  drastic"). The next look is **Area** (area.tech case studies); ask Mikey for reference screenshots. Release run 58:
  UI 2555/0, but Codex approval harness tests and engine message_queue tests flake only in the full cross-crate run
  (pass alone). Fix in progress: item ffa6f957, branch `fix-release-flakes`.
- **2026-11-06 evening (release 60, installed):** Style and theme are INDEPENDENT (Mikey's model). A style
  changes element design only (shapes, control variants, structure, type, status shape), and the theme supplies
  the colours, so any combination works. `style-theme-decouple`: no forced theme on style switch; Industrial
  Light/Dark ship as ordinary themes; theme-derived hardware roles. Bold Industrial phase 2: round/pressed keys,
  attached ledges, green confirm (theme success role), dark mono readouts (model/context), MODEL/MODE and YOU/AGENT
  faceplate labels, grille texture, chassis bezel, notched toggles. `looks-area`: the Area style from the
  area.tech OpenAI case-study interface (rounded, identity hue per agent with a calm-first palette that avoids
  danger, filled user pills, serif agent replies (EB Garamond, OFL), harness icon tabs on a filled composer, SEND
  key, dark rounded code/tool panels, artifact nav/footer) + Area Light/Dark themes; notes in
  docs/internal/styles/area.md. `ui-preview`: `scripts/ui-preview.sh [--themes …] [--screens …]` renders
  screens × styles × themes into labelled contact sheets (docs/internal/ui-preview.md). UI workers must look at
  and attach a sheet before handoff; Codex needs one approval to run it outside the sandbox, and the coordinator
  can run it directly. `tool-error-friction`: id prefixes for requests and items, full ids in worker banners,
  scope diagnostics, facts/links truncate with warnings, and a `dismiss_friction` tool. After this install, dismiss
  friction 8e01dbe19385db7a364f1e5b84c6d31a, 4a607d99fdef1ef4c28d2dfc90a303b9, e24a833369cf43dff6c1f9b978d9db79 and
  a865ed1a524c1577bbc1a0974a625125 (already fixed). `fix-release-flakes`: SessionDoc serializes queue mutations; a
  real append-versus-drain race rejected queued messages. Gallery: T cycles themes. Release 60: UI 2563/0, plus one
  engine test updated for truncate-not-refuse. Running: `approval-rules` (58571810): per-Space auto-approve
  patterns for worker command approvals; suggestions off by default; 39 adversarial cases must ask.
- **2026-11-06 late (release 61, installed, clean: nextest 2477/2477, UI 2569/0):** `approval-rules`: per-Space
  auto-approve rules for Codex command approvals. A fail-closed matcher unwraps zsh -lc, perl alarm, caffeinate,
  ulimit and env prefixes; every segment of a chained command must match; 39+ adversarial cases always ask;
  cargo test/nextest/build/check/clippy are allowed, while install/publish/login/add/update and registry/net config
  always ask. Durable friction audit and counts; Space settings editor with a test box; "Always allow commands like
  this…" on approval cards; agents can only PROPOSE rules (one-click accept). Suggested rules ship OFF. Codex only;
  Claude Code permission prompts don't go through it. Mikey should enable the suggestions.
- **Appearance model (Mikey, 2026-11-06): three independent settings.** Style {Default, Industrial, Area} = element
  design; Layout {Minimal, Compact, Guided} = density and guidance; Theme = colours. Any combination must work. In
  progress: `ui-components` (f08a32a7), a shared component layer so screens never branch on style/layout, plus a
  ratchet test on literal sizes/radii and style branches. Then migrate screens in slices (see
  docs/internal/ui-components.md). Area v2 and Industrial polish are PAUSED until the components land. Mikey's
  verdict: both looks are "inconsistently applied", and Area "all over the place".
- Release runs 50–55 passed (last: nextest 2405/2405, UI 2542/0). Run 54 failed two load-sensitive tests
  (`message_queue::queued_text_waits_for_a_steerable_turn_even_with_legacy_policy`,
  `executable::tests::stderr_versions_are_read_and_build_metadata_does_not_break_ties`) while worker builds ran
  alongside. Both pass alone 3/3 and in run 55; quarantine them if they recur.
- For Mikey's morning review (Required Actions): workflow pack (mast-artifact:771f405aaf35fc7192f309027ced0a95),
  coworker readiness (mast-artifact:0497fc4075db3cd988c9c69f8e08d6de; decisions: clean-Mac tester, ad-hoc vs
  notarised DMG, delete the stale `archive/ios` + `audit-*` origin branches), UI styles
  (mast-artifact:350fe811d51a2309281fbac0be7f6981, questions A–H).
- Load: five concurrent builders plus a release run pushed load to ~98 on 18 cores. Keep it to about 3 builders plus
  1 release run.

**Observing (minimal-talk phase 0, approved 2026-11-05):** item 9516dd7c, worker chat 4ccb0a0d (archived).
Baseline for the warehouse dashboard, with Basedash, Kit Line and StockLot as comparisons, is in the observation log
(mast-artifact:0b48afa290246f61b51f8ca21765e793), rev 2. Window 2026-11-05 → 11-12. Around 11-12, take a fresh
read-only snapshot and APFS-clone it into `.worktrees/phase0-data/` (gitignored; Codex asks approval for every read
outside its workspace). Then have a worker follow the log's re-run steps and produce the go/no-go. Still unmeasured:
worker runtime status and PR/Linear sync visibility. Delete `.worktrees/phase0-data/` after.

**Waiting on Mikey:**
- Active-checkout tracking design (Needs-you, parked item 40377ea3).
- Proposed instruction edit on Mast Dev (install step → install script, proposal acd9f3ef): accept it.
- Cofactr: accept Space-root proposal 8838b1f4; admin-chat follow-ups (Q-7, archive the old Project Template copy,
  Tickets rules rev 3, ENG-3222).
- `peon_hook.py`: still called (3 hooks in ~/.claude/settings.json, 6 in ~/.codex/hooks.json, and Codex's notify
  `--previous-notify` chain), but it posts to port 8765, where nothing listens since the tray was retired. Removing
  the hook entries needs Mikey's OK, because agents don't edit ~/.claude or ~/.codex. Then delete the script, its
  ~/bin symlink and /tmp/peon_hook.log.

**Queued (agreed):**
1. Coworker readiness: workflow pack (export Mikey's instructions/templates/model routing as an optional importable
   pack; draft for Mikey first), then a real coworker dry run (Mikey picks who).
2. Retire the agent-mode-tools wrapper (Space root → plain clone of Pluto8195/mast, worktrees under it, move the
   `.zeron/`/`.mast/` artifact folders incl. the brand kit, update root instructions/HANDOFF/memory; drops the
   submodule-bump step). Needs a quiet window with no workers.
3. Later: PR review phases 5–7 (drafts/threads, post to GitHub, re-review), item-tab artifact surfacing design (171cde5a), container templates phases
   2+, one attention model / one chat-status model (design first), the cross-device archive race follow-up from
   stale-coordinator-wakes, optional "move ~/.zeron to ~/.mast" Settings action.
4. Known bug to file/fix: Claude Code's multi-question ask-user prompt in Mast returns "rejected" and drops answers
   (friction 8e01dbe1); ask in plain text meanwhile.

**Terra working style:** it ends turns early ("can't complete in this turn") on anything that looks big.
Give one or two concrete items per chat, say "partial progress is fine: commit and keep going", start a
fresh chat per slice on the same branch, and archive the finished one. Build with
`CARGO_TARGET_DIR=<worktree>/target-clean`; never seed the cache. Approve its unsandboxed test reruns
(watcher/home-dir tests only pass outside Codex's sandbox). Codex shells default to 256 open files: tell
workers to `ulimit -n 4096` before tests; that fixes most nextest stalls and agent_tools failures. Have them
complete their item BEFORE raising Ready to merge (an item with an open request can't be completed).

**Disk:** prune `zeron/target/debug` when idle (`find deps -maxdepth 1 -type f -mmin +360 -delete`, same for
build/ and .fingerprint/); delete merged worktrees' target-clean right after merging. Repo auto-gc is off.

**Cofactr admin-chat follow-ups Mikey is sending:** resolve stock lot Q-7, archive the old Project Template copy,
Tickets rules rev 3 (`kind: review` until the PR is approved, then `handoff`). Cofactr Space-root proposal
8838b1f4 (kinds/facts/links) awaits Mikey's accept.

**What Mikey is doing next:** using an admin chat in the Cofactr Space to set up the Projects container
template (matching his current copy template), move existing projects under it with a preview, and
run the stage-0 talk-budget pilot from the minimal-talk research (mast-artifact:37a6c5c807eb8fe30aa1b56044a8749a).

**First admin chat (Cofactr Space): how and the drafted prompt.** Start a chat in the Cofactr Space →
right-click it → "Grant admin for this Space…" (4 h, "Skip bulk confirmations" OFF the first time).
Prompt, to adjust with Mikey:

> Set up live container templates for my projects:
> 1. Create a **Projects** item (or use the existing one) and configure its container template to
>    reproduce my current copy project template exactly — same child items, same instructions per node,
>    Tickets as the worker destination. (Find the current template in Space-root details → Templates.)
> 2. Create a test project under it with New work item and check it has the full structure and live
>    instructions; archive it afterwards.
> 3. Move my existing projects (Basedash, Kit Line, the warehouse dashboard, the stock lot traceability
>    one) under Projects and apply the template to each, mapping existing children by title. Show me the
>    preview as a Required Action before applying.
> 4. Apply the stage-0 "talk budget" pilot from the minimal-talk research
>    (mast-artifact:37a6c5c807eb8fe30aa1b56044a8749a) to the warehouse dashboard's Planning/orchestrator
>    instructions, as instruction edits I can see in the action log.
> 5. Report back in ≤10 lines: what changed, what didn't map cleanly, decisions I need to make.
> Rules: archive, never delete; bulk changes go through the preview → Required Action; everything must
> be undoable from Recent Changes.

**Waiting on Mikey (Required Actions open):** PR review brief (mast-artifact:3e12f5d0be4cb1de57ed6b0f8be58992),
active-checkout design
(mast-artifact:679f61de127b3519c16de37f8d90f99b), `mast-doctor` Ready to merge.

**Coordinator lessons from this session:**
- Design-only children never push, so check Required Actions whenever a child goes idle. Orchestrator
  wakes now deliver these as "Worker updates".
- When several branches conflict, push an integration snapshot as `release-staging` and have the
  children rebase onto it. Merge the most foundational one first.
- Session and Mast restarts kill background shells: re-arm watchers, resend children, rerun tests.
- When you say "right after", queue it in the same step.
- Never start long runs under `nice`. Children must only kill processes inside their own worktree.
- Logs: `~/.zeron/logs/mast-headed.log`.

## Known gaps

- Visual compare / zoom (after `compare-zoom-gaps`, 2026-10-28):
  - Render diffs are capped at 16,000 CSS px (no tiling); longer pages show "Compared the first
    16,000 px of N". Unverified in a real WKWebView at that height.
  - HTML/Mermaid compare zoom mirroring is headless-tested only. Onion skin doesn't zoom.
  - Option revisions recorded before 2026-10-20 can't be compared across revisions.
- Linear GraphQL (projects, attachments) unverified against the real API.
- **Usage (2026-10-27):** a 7-day audit of `~/.claude/projects/*.jsonl` found about 43% of usage
  in three long-lived coordinator chats (Opus median context 226K tokens, p90 680K). Keep
  coordinators thin, restart them often (this file holds the state), and keep screenshots and
  design rounds in child chats. The unexpected Fable usage (about 18%) is fixed: see "Every chat
  start passes an explicit model" above.

## Next, agreed but not started

- **Container templates phases 2+** (plan mast-artifact:df5be7ef1a2fa657e3737f17dd0ef89a), after Mikey's
  admin-chat test shows what is missing.

## Parked (do not start unasked)

Archived-chat retention (measured 2026-10-30, not needed yet): 363 chats, 108 archived; archived rows
are 0.33 MB of the 6.4 MB registry, archived transcripts 37 MB of 93 MB snapshot docs; `docs.sqlite3`
112 MB, journals 265 MB (626 files). Cost is mostly disk; boot reads journal tails only. If it grows,
add a retention setting that compacts/exports transcripts and journals of chats archived > N days,
keeping the registry row.

Decisions 2026-10-28: Linear automatic project sync / status write-back stays parked (manual
sync is fine; auto changes could make a mess). The internal zeron → mast code rename goes on its
own branch `rename-zeron-to-mast`, not merged until Mikey reviews it. Active-checkout tracking
stays parked.

See `parked-proposals.md`: active-checkout branch tracking, Linear v2 automatic project sync and
status write-back, a "Waiting for input" group (declined), Tod-style action log / undo.

## How we work

- **Worktrees:** `<home>/Projects/agent-mode-tools/.worktrees/zeron/<branch>`, branched from
  the integration head. Seed the build cache with
  `cp -c -R target/debug/{deps,build,.fingerprint}` (APFS clones), add a `.workspace-root` file,
  and write `.cargo/config.toml` with `[build]\nincremental = false` (untracked; `.cargo/` is in
  info/exclude). Incremental caches were roughly 20 GB per worktree.
  Merge with `--no-ff` into `work-outline-implementation`. **Delete each worktree's `target/` after
  merging**; keep at least 100 GB free. **At most 2 builds at once in total, counting the coordinator's own release
  run**: release only while a worker is idle, or pause one first. Worktree `.cargo/config.toml` also sets `jobs = 8`
  under `[build]`. On 2026-11-16, 4 concurrent builds (load 44 on 18 cores) made Mast lag badly.
  Since 7c53f6b54, dev builds use line tables only and no dependency debug info.
  If disk still nears 100 GB, first run `cargo sweep --time 3` in the main checkout (cargo-sweep is
  installed; it removes build outputs not touched in 3 days and freed 69 GB on 2026-11-14), then clear
  `target/debug/incremental` and `target/installed`; both rebuild.
- **Subagents:** one narrow task per agent with clear file ownership; read files in ≤200-line
  slices (they stall on huge files like `overview.rs` / `shell.rs`); commit early and often.
- **Tests:** release runs use `caffeinate -i scripts/run_release_tests.sh [BASE_REF]` (nextest
  `release` profile for non-UI crates with a 3-min per-test timeout and the known flaky tests
  quarantined; UI via `cargo test -p mast-ui --lib -- --test-threads=1`; only the crates the diff
  touches, `--all` for everything; `.config/` or Cargo changes select all). Any failure there is new.
  `--quarantine` runs the flaky set separately. Always `caffeinate -i`: the Mac's maintenance sleep
  pauses long runs. Details and timings: `release-testing.md`. Build the installed binary in parallel
  with a plain `cargo build --profile installed -p mast` (incremental `target/installed`).
- **Install:** `cargo build --profile installed -p mast` (optimised; since 2026-10-27, replaces the
  debug build); back up the current binary to `~/zeron-dev-app-backups/`;
  `scripts/install-macos-app.sh target/installed/mast /Applications/Mast.app computer.mikey.mast.dev` (atomic
  binary copy to `Contents/MacOS/mast`, `zeron` symlink, Info.plist executable/id/URL schemes, ad-hoc codesign,
  lsregister); push `work-outline-implementation`. Before 2026-11-05 this was a `cp … Contents/MacOS/zeron` plus
  `codesign --identifier sh.zeron.app.dev`. Earlier bundles:
  `Zeron Dev.app` (until 2026-10-23) and `Creel.app` (until 2026-10-26), kept only as rollbacks.
- **Perf gate before every install** (Mikey, 2026-11-16: perf is part of review, before a restart):
  1. Branch review includes a perf pass on the diff: per-Space/worktree/chat loops that run git or I/O (cached and
     bounded?), timers and polls, retry loops (backoff?), `cx.notify()`/redraw on ticks, whole-list scans in render.
  2. The release suite's budget tests (engine `idle_spawn_budget`, UI PR-link index and retry budgets) must pass at
     real scale.
  3. `scripts/perf-soak.sh target/installed/mast --baseline /Applications/Mast.app/Contents/MacOS/mast --real`:
     sandboxed headless engine soak (MAST_SOAK=1, no network, no writes, git guard) on a `.backup` copy of the data.
     Worse CPU, git spawns/min or footprint growth = no install.
  `scripts/install-macos-app.sh` enforces 2 and 3 via stamps in target/perf-gate/ (scripts/perf-gate.py): it refuses a
  binary without a passing release-test stamp for HEAD and a passing soak of that exact binary. `--skip-perf-gate
  "<reason>"` only with Mikey's OK; it appends to docs/internal/perf-gate-overrides.md, which must be committed.
- **Perf check after every release** (post-mortem 2026-11-16, P6): about 2 minutes after Mikey restarts, run
  `scripts/perf-sample.py` and record CPU, physical footprint and the top frames in the snapshot (RSS includes GPU
  maps and overstates by GBs). Steady CPU above ~25%, or footprint
  growing more than 1 GB in 15 minutes, blocks further feature releases until explained.
- **Perf review rules** (same post-mortem, P4/P5): no fixed-interval retry loops (back off; "not found" and bad
  params are non-transient); any per-Space, per-worktree or per-chat loop that runs git or I/O is cached and bounded;
  render paths never scan whole snapshots or chat lists per row (precompute per revision).
- **Never restart Mast automatically.** Install, then tell Mikey what to restart and test.
  Restarting kills agents running inside Mast, so commit WIP first.
- **Xcode license prompts** after Xcode updates block git/cargo: Mikey accepts, or use
  `DEVELOPER_DIR=/Library/Developer/CommandLineTools`.

## Hard rules for agents

- No GUI launches, example binaries, or screen capture; verify UI with headless gpui tests.
- Never print secrets; never touch `~/.claude` / `~/.codex` config or sessions, or Codex
  computer-use.
- Never write to live `~/.zeron` stores; inspect with read-only sqlite `.backup` copies.
- No real Linear/GitHub API calls in tests. Nothing is ever written to Linear.
- Never commit to `main` directly; the coordinator fast-forwards it to the integration head after each push.
- Registry operation ids must be ≤ 64 characters; hash long keys with `derive_id`.
- **The Mast repo is public.** Never write these into it (docs, commits, fixtures):
  - device IDs (UDIDs), team IDs, account logins or emails
  - Tailscale names or IPs
  - home paths: use `<home>`
  - employer, MDM or DevOps specifics

  Put real values in the private parent repo's `docs/private/` (e.g. `docs/private/ios-device.md`) and point to
  them. Run `gitleaks git --redact --log-opts=--remotes=origin` before a release push if in doubt.

## Diagnostics

- Outline key handling: grep `work_outline_keys` in `~/.zeron/logs/mast-headed.log` (zeron-headed.log before the rename).
- Linear PR attachments: log target `linear`.
- Pre-settle registry DB backup:
  `~/zeron-dev-app-backups/local-profile-docs-2026-10-12-pre-settle.sqlite3`.
