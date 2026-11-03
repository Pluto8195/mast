# Audit of remaining Zeron and Creel names

Audit date: 2026-11-03. Base: `d40adecf8` (`origin/work-outline-implementation` after fetch). Audit only; no product changes, builds, tests, live databases or GUI. The existing [migration plan](../rename-migration.md) contains useful prior design, but is a proposal, not evidence that every proposed migration shipped.

## Scope and counting

All tracked files were enumerated with `git ls-files`; UTF-8 text was scanned case-insensitively for `zeron|creel`, including hidden files and workflows. Counts are **matching lines**, not occurrences, unique identifiers or tasks. A line is assigned once using the precedence described below; a line may contain multiple meanings. Paths are counted separately, including binary assets; binary contents, ignored/untracked files, git history, remote services, OS registrations and live data were not searched. The line-level index below is exhaustive for this tracked-text scan and is the review checklist. Bucket assignment is a conservative automated triage, not a claim that every line has been individually adjudicated: inspect its context before editing. In particular, visible CLI instructions that name a real binary are compatibility-dependent, and upstream landing-page quotes are attribution, not copy to rewrite.

Precedence: upstream URLs/attribution, parity references and historical plan/research documents → 4; legacy-looking protocols, paths, domains, env vars and serialized names → 3; remaining landing/iOS/docs/distribution material or string literals → 1; remaining code/comments → 2. This deliberately overcounts migration and visible review candidates and undercounts history elsewhere. Category 2 includes tests and comments, many of which should remain as compatibility fixtures or upstream attribution. Category counts must not be treated as implementation estimates.

| Triage category | Matching lines |
|---|---:|
| 1 Visible/review | 384 |
| 2 Internal/review | 408 |
| 3 Migration/compatibility | 888 |
| 4 Keep/history | 598 |

Total: **2278 matching lines**, in **365 files**; **101 tracked paths** contain a name (including files under named directories).

### Counts per crate and other areas

| Area | Visible/review | Internal/review | Migration | Keep/history | Total |
|---|---:|---:|---:|---:|---:|
| .github | 1 | 3 | 16 | 1 | 21 |
| ARCHITECTURE.md | 0 | 15 | 7 | 2 | 24 |
| Cargo.lock | 0 | 0 | 0 | 1 | 1 |
| Cargo.toml | 0 | 0 | 0 | 6 | 6 |
| README.md | 1 | 0 | 1 | 1 | 3 |
| THIRD_PARTY_NOTICES.md | 0 | 6 | 1 | 3 | 10 |
| apps/ios | 113 | 0 | 55 | 3 | 171 |
| apps/landing | 18 | 0 | 32 | 5 | 55 |
| apps/mast | 34 | 17 | 22 | 0 | 73 |
| apps/www-redirect | 0 | 0 | 4 | 0 | 4 |
| crates/doc | 0 | 10 | 2 | 0 | 12 |
| crates/engine | 48 | 59 | 153 | 3 | 263 |
| crates/harness | 10 | 27 | 67 | 6 | 110 |
| crates/mcp | 0 | 10 | 65 | 0 | 75 |
| crates/proto | 0 | 22 | 68 | 0 | 90 |
| crates/rpc | 0 | 7 | 1 | 0 | 8 |
| crates/sync | 0 | 4 | 2 | 0 | 6 |
| crates/syntax | 2 | 0 | 0 | 0 | 2 |
| crates/theme | 0 | 4 | 9 | 2 | 15 |
| crates/ui | 75 | 185 | 188 | 16 | 464 |
| crates/update | 11 | 3 | 37 | 0 | 51 |
| dist | 13 | 0 | 13 | 0 | 26 |
| docs | 27 | 0 | 66 | 548 | 641 |
| edge | 17 | 5 | 36 | 0 | 58 |
| scripts | 14 | 31 | 43 | 1 | 89 |

## Most visible candidates

These are prioritized surfaces, not a claim that every surface ships in the desktop release. Landing site and iOS are upstream-derived and should be scoped explicitly before publishing. Command examples must stay accurate until the binary changes.

| # | Location | Exposure / action |
|---:|---|---|
| 1 | `apps/landing/public/index.html:6` | Browser title says Zeron. |
| 2 | `apps/landing/public/index.html:7` | Search description says Zeron. |
| 3 | `apps/landing/public/index.html:8` | Social preview title. |
| 4 | `apps/landing/public/index.html:779` | Home link accessibility label. |
| 5 | `apps/landing/public/index.html:781` | Header wordmark. |
| 6 | `apps/landing/public/index.html:872` | Screenshot alt text. |
| 7 | `apps/landing/public/index.html:934` | Product introduction. |
| 8 | `apps/landing/public/index.html:944` | Session screenshot alt text. |
| 9 | `apps/landing/public/index.html:948` | Diff screenshot alt text. |
| 10 | `apps/landing/public/index.html:952` | History screenshot alt text. |
| 11 | `apps/landing/public/index.html:1010` | Testimonials heading; preserve actual quoted posts. |
| 12 | `apps/landing/public/index.html:1012` | Testimonials accessibility label. |
| 13 | `apps/landing/public/index.html:1409` | Download heading. |
| 14 | `apps/landing/public/index.html:1415` | Desktop download heading. |
| 15 | `apps/landing/public/index.html:1427` | Mobile download heading. |
| 16 | `apps/landing/public/index.html:1445` | Footer product description. |
| 17 | `apps/landing/public/index.html:1456` | Footer wordmark. |
| 18 | `crates/ui/src/shell.rs:5973` | Error instructs `zeron daemon stop`; update alongside binary compatibility. |
| 19 | `crates/ui/src/shell.rs:8305` | Update notice instructs `zeron update`; same dependency. |
| 20 | `apps/mast/src/main.rs:354` | CLI error asks whether zeron is running. |

Installer assets and URLs are not merely copy: `scripts/package-macos.sh:25`–34 produces old-named release files and a `Zeron.app` update payload, and landing downloads point at upstream `zeron.sh` (`apps/landing/public/index.html:1419`). Verify ownership and release infrastructure before sharing; do not redirect coworkers to upstream binaries. iOS product/bundle naming (`apps/ios/Zeron.xcodeproj/project.pbxproj:137,324`) is a separate distribution decision.

## Compatibility boundaries and proposed migrations

Effort estimates are engineering order-of-magnitude, excluding release operations and external account setup. Verify on disposable profiles and mixed-version clients; never run migrations against Mikey's live data as part of this audit.

| Boundary | Existing-install impact | Approach / effort / risk |
|---|---|---|
| Executable `zeron`, CLI, daemon and release archive | Existing scripts, MCP commands, service units, updater and `CFBundleExecutable` expect it (`apps/mast/Cargo.toml:9`, `apps/mast/src/daemon.rs:15`, `scripts/package-macos.sh:25`). | Ship `mast` plus a working `zeron` compatibility executable/shim; update plist, packaging, service install, updater and docs together. Old updater expects `Zeron.app` payload: retain until a bridge release can read both. Test rollback and upgrades; M–L, high. |
| Bundle id / notifications / signing | Changing `sh.zeron.app.dev` changes OS app identity; permissions, preferences, notification behavior and keychain access may require reapproval. Source notification constant is `sh.zeron.app` (`crates/ui/src/notify.rs:77`), distinct from installed dev id; verify actual bundle and notification behavior. | Choose a domain controlled by the distributor; migrate preferences where needed, re-sign, document permission prompts and test clean install and upgrade. Do not assume grants or ACL behavior is uniform; M, high. |
| Data root, logs, worktrees | `~/.zeron` is current default (`apps/mast/src/paths.rs:33`); Windows uses `Zeron` too. A blind move can strand absolute worktree paths, git metadata, scripts and running processes; logs follow the root. | New installs use `.mast`; existing installs first retain `.zeron`, with explicit offline locked move and durable old-path symlink only after verifying both paths and backups. Resolve both-present, symlink, cross-volume, failure and rollback cases; M–L, high. |
| Linear keychain service | `sh.zeron.linear` (`crates/engine/src/linear/credentials.rs:13`) lookup would miss Mikey's saved key. | Read new service first, fallback to old, copy only on successful access and leave old item for rollback; account for OS access prompts. S–M, medium. |
| URL schemes and auth | Desktop generates and parses `zeron://` (`crates/ui/src/links.rs:44`); pasted links and iOS OAuth callback (`apps/ios/Zeron/Info.plist:29`) can break. | Register/read both schemes; emit new links only after all consumers accept them. Coordinate OAuth allowlists and iOS identities; S–M desktop, L including hosted auth, high. |
| Environment, project files, artifacts, HTML bridges | User scripts, agents, saved plans and old devices use `ZERON_*`, `zeron.json`, `.zeron/artifacts`, `zeron-artifact:`, `zeron-file:`, `data-zeron-*`, bridge messages. | Existing aliases are described in prior plan; verify both read and write paths and precedence. Keep legacy reads and child exports; retain old artifact paths for existing items. Rollout tests must cover old/new writers; M, medium–high. |
| Registry, wire, transcript and hash names | `zeronOwned`, `zeronUpgradeRequired`, instruction envelope and idempotency namespace can be persisted or interpreted across devices. Changing hashes/keys can duplicate operations or break sync. | Keep wire names and golden hashes; introduce dual readers before changing any writer and prove mixed-version behavior, or keep indefinitely. M–L, high. |
| MCP server / agent sessions | New runs use `mast-agent` but old runs/transcripts may refer to `zeron-agent`; changing injected executable breaks active sessions. | Keep both recognition/rendering and env aliases; preserve active configs and old executable until sessions complete. Test Claude and Codex configs and transcript replay. S–M, medium. |
| Services, updates, domains | launchd `sh.zeron.app`, systemd `zeron.service`, update manifest and `edge.zeron.sh` are operational contracts; an upstream endpoint may serve upstream code or require credentials. | Inventory ownership, deploy controlled infrastructure, preserve old readers/units, migrate with explicit user choice and rollback. Verify auth, update signatures and no cross-install. L, high. |
| iOS identity, keychain and callback | Bundle `sh.zeron.ios` and callback are separate from desktop; switching can isolate app storage, entitlements and App Store identity. | Decide whether mobile is in scope; design a separate export/import and registered App ID/auth rollout. L, high. |

Current `.mast` aliases do not make all mixed versions symmetric: the prior plan notes older clients cannot see newly created `.mast` artifact folders. Keep legacy test fixtures even after code identifiers change. No databases were inspected, so actual stored values and usage are unknown.

## Internal names and deliberate retention

Category 2 is a broad mechanical-review pool, not a promise of a globally safe substitution: private fields such as `self.zeron` in `crates/mcp/src/tools.rs` are straightforward; `zeron_conversation_link`, `TERM_PROGRAM`, log targets, serialized names and tests may cross a boundary. Rename local identifiers in small crate-scoped changes, keep golden compatibility tests, and separately assess log filters and scripts. Crates were already renamed; do not repeat that change. The line index gives exact scope by crate.

Keep upstream dependency URLs, license notices, true upstream quotations, history and `upstream` remote `https://github.com/zeronsh/zeron.git`. Keep `zeron/` submodule and `.worktrees/zeron/` until a coordinated quiet window: git worktree metadata, bindings, root instructions and active chats use absolute paths. Historical HANDOFF and specifications are records, not current onboarding; annotate only if misleading. Preserve compatibility aliases and old fixture values for as long as they are accepted. Renaming the parent path is optional and has little coworker-facing value.

The tracked superproject files containing either spelling outside the submodule are `.gitmodules`, `peon_hook.py`, `README.md`, `MIGRATION-HANDOFF.md`, `tickets/001-import-external-sessions.md`, and `tickets/002-multichat-overview.md`. `peon_hook.py` is the only tracked script in that list; inspect its actual references before deciding whether it needs a path change. Ignored/untracked superproject scripts and live work-outline instructions were not inventoried.

## Suggested separately mergeable branches

1. **Desktop copy and onboarding:** correct visible product strings and current docs, leaving functional legacy commands accurate; explicitly preserve attribution. Verify UI/headless text tests and doc links.
2. **Distribution safety:** establish owned release/update endpoints and packaging identity; stop upstream download references from being presented as Mast. Validate artifacts, checksums, signing/notarization and clean installs in a controlled release. Decide whether website/iOS are in scope.
3. **Internal identifiers:** crate-scoped mechanical changes and test updates, excluding persisted values, protocol strings and useful upstream references. Run affected crate tests.
4. **Compatibility foundation:** add missing dual readers, mixed-version fixtures and telemetry-free diagnostics; preserve old writers until old clients can read new forms. Cover links, environment, projects, artifacts, registry and agent configs.
5. **Binary and services:** introduce `mast` plus old executable compatibility, updater bridge and service migration, then user-facing command changes. Exercise upgrade and rollback.
6. **Data and keychain:** new-install defaults plus old-root fallback and keychain dual read; defer optional offline data move until explicitly exercised and approved. Test collision and interrupted migration.
7. **Bundle identity and schemes:** choose owned identifiers, register both schemes, migrate preferences as needed and document permission reauthorization. Coordinate a user-performed restart and test.
8. **Optional mobile and submodule path:** separate mobile identity/auth rollout from a quiet-window parent repository/worktree rename. Neither blocks a macOS coworker package if excluded deliberately.

Before implementation decide distribution domain/signing identity, desktop-only versus iOS/web scope, updater hosting, coexistence/rollback policy and how long legacy commands are supported. A branch is mergeable only with its own compatibility tests; several steps need staged releases before removing old behavior.

## Complete tracked-text index

Each entry lists every matching line number in that file, grouped by the triage bucket above. The same file can appear in multiple buckets. Binary file contents were not scanned. Line numbers refer to the audited base, not future edits.

### 1 Visible/review

- `.github/workflows/windows.yml`: 100
- `README.md`: 50
- `apps/ios/README.md`: 3, 70, 71
- `apps/ios/Zeron.xcodeproj/project.pbxproj`: 10, 15, 20, 23, 25, 28, 30, 33, 35, 46, 53, 60, 65, 72, 73, 97, 98, 99, 108, 109, 117, 119, 130, 132, 137, 141, 143, 153, 155, 158, 159, 162, 164, 174, 176, 179, 180, 198, 216, 217, 218, 392, 428, 435, 444, 453, 462
- `apps/ios/Zeron.xcodeproj/xcshareddata/xcschemes/Zeron.xcscheme`: 19, 36, 37, 46, 47, 69
- `apps/ios/Zeron/App/DemoDataset.swift`: 40, 41, 110, 119, 120, 130, 144, 274
- `apps/ios/Zeron/App/E2ERunner.swift`: 2
- `apps/ios/Zeron/App/ZeronApp.swift`: 1, 8
- `apps/ios/Zeron/Composer/Attachments.swift`: 2
- `apps/ios/Zeron/Info.plist`: 32
- `apps/ios/Zeron/Sync/ChatFrames.swift`: 5
- `apps/ios/Zeron/Sync/DocDisk.swift`: 42
- `apps/ios/Zeron/Sync/RegistryCore.swift`: 9
- `apps/ios/Zeron/Theme/Motion.swift`: 19, 24
- `apps/ios/Zeron/Views/Loaders.swift`: 102
- `apps/ios/Zeron/Views/NewSessionView.swift`: 110
- `apps/ios/Zeron/Views/SessionView.swift`: 42
- `apps/ios/Zeron/Views/SignInView.swift`: 3, 16, 45, 238, 241, 255, 261, 267
- `apps/ios/ZeronTests/AppshotTests.swift`: 2
- `apps/ios/ZeronTests/ChangeRequestTrackingTests.swift`: 2
- `apps/ios/ZeronTests/ChatFramesTests.swift`: 6
- `apps/ios/ZeronTests/ComposerEditorTests.swift`: 3
- `apps/ios/ZeronTests/DeviceRelayClientTests.swift`: 2
- `apps/ios/ZeronTests/GeneratedImageTests.swift`: 5
- `apps/ios/ZeronTests/HarnessCatalogTests.swift`: 2
- `apps/ios/ZeronTests/MessageQueueTests.swift`: 6
- `apps/ios/ZeronTests/NetworkReliabilityTests.swift`: 7
- `apps/ios/ZeronTests/PinnedSessionsTests.swift`: 2
- `apps/ios/ZeronTests/ProjectlessSessionTests.swift`: 3
- `apps/ios/ZeronTests/RegistryCoreTests.swift`: 7
- `apps/ios/ZeronTests/RegistryDocTests.swift`: 5
- `apps/ios/ZeronTests/SessionStoreDurabilityTests.swift`: 3
- `apps/ios/ZeronTests/SessionStoreLifecycleTests.swift`: 2
- `apps/ios/ZeronTests/StoreResidencyTests.swift`: 2
- `apps/ios/ZeronTests/TranscriptFollowTests.swift`: 2
- `apps/ios/ZeronTests/TranscriptLayoutTests.swift`: 4
- `apps/ios/ZeronTests/TranscriptPresentationTests.swift`: 2
- `apps/ios/ZeronTests/TranscriptScrollMatrixTests.swift`: 4
- `apps/ios/ZeronTests/TransportErrorDescriptionTests.swift`: 3
- `apps/ios/ZeronUITests/MobilePolishTests.swift`: 140, 247
- `apps/ios/ZeronUITests/ProjectlessSessionUITests.swift`: 75, 89, 93, 94, 102
- `apps/landing/public/index.html`: 6, 7, 8, 779, 781, 800, 872, 934, 944, 948, 952, 1012, 1080, 1180, 1415, 1427, 1445, 1456
- `apps/mast/Cargo.toml`: 9
- `apps/mast/build.rs`: 43
- `apps/mast/src/auth_cli.rs`: 65, 72, 97, 102
- `apps/mast/src/daemon.rs`: 39, 70, 111, 120, 132, 145, 169, 183, 185, 212, 383, 387, 390, 394, 412, 413, 430, 433, 440, 446
- `apps/mast/src/main.rs`: 16, 354
- `apps/mast/src/paths.rs`: 28, 70, 79, 88
- `apps/mast/src/update_cli.rs`: 16, 21
- `crates/engine/src/agent_tools.rs`: 2202, 2216, 2224
- `crates/engine/src/change_requests.rs`: 559
- `crates/engine/src/doc_host.rs`: 6294, 6295
- `crates/engine/src/execution_scope/tests.rs`: 25
- `crates/engine/src/instance_lock.rs`: 64, 169
- `crates/engine/src/lib.rs`: 1346, 1361
- `crates/engine/src/pr_ticket_cache.rs`: 1528
- `crates/engine/src/repos.rs`: 74
- `crates/engine/src/source_control.rs`: 1472, 1610, 1640
- `crates/engine/src/terminals.rs`: 280
- `crates/engine/src/terminals/windows.rs`: 322
- `crates/engine/src/titles.rs`: 673, 701, 718, 735, 741, 744, 745, 749, 760, 780
- `crates/engine/tests/agent_tools.rs`: 1279
- `crates/engine/tests/diff_targets.rs`: 18
- `crates/engine/tests/diff_targets_sync.rs`: 23
- `crates/engine/tests/external_import_auto_adopt.rs`: 111, 112, 130
- `crates/engine/tests/external_import_junk_filter.rs`: 188, 208
- `crates/engine/tests/external_import_title_repair_harness_session.rs`: 72, 73, 75, 109, 113
- `crates/engine/tests/m5_repos_diffs_terminals.rs`: 225
- `crates/engine/tests/pr_review_launch.rs`: 41
- `crates/engine/tests/pr_review_sync.rs`: 45
- `crates/engine/tests/ref_diff_targets.rs`: 30
- `crates/engine/tests/work_outline_execution_rpc.rs`: 41
- `crates/engine/tests/work_outline_launch.rs`: 50
- `crates/engine/tests/work_outline_pr_links.rs`: 34
- `crates/harness/src/acp/mod.rs`: 1948
- `crates/harness/src/acp/subagent.rs`: 685
- `crates/harness/src/adapter_install.rs`: 93, 98, 696, 697, 723, 734
- `crates/harness/src/opencode/mod.rs`: 580
- `crates/harness/tests/fixtures/fake-acp.sh`: 28
- `crates/syntax/src/lib.rs`: 1180, 1184
- `crates/ui/src/app_menus.rs`: 251, 255
- `crates/ui/src/appshots.rs`: 147, 1226
- `crates/ui/src/artifacts/comments.rs`: 439, 440
- `crates/ui/src/build_watch/tests.rs`: 33
- `crates/ui/src/changes.rs`: 8228
- `crates/ui/src/chat_metadata.rs`: 475
- `crates/ui/src/image_media.rs`: 287
- `crates/ui/src/lib.rs`: 227, 464
- `crates/ui/src/overview.rs`: 759, 7123, 7467, 7487, 7508, 7586, 7610, 7622, 7640, 7822, 8292, 8369
- `crates/ui/src/overview_grouping.rs`: 222, 227, 275, 295, 333
- `crates/ui/src/overview_positions.rs`: 103
- `crates/ui/src/pickers.rs`: 6991, 6992, 7013, 8069, 8070, 8071, 8132, 8141
- `crates/ui/src/settings.rs`: 1799, 1812
- `crates/ui/src/shell.rs`: 5973, 8305, 9365
- `crates/ui/src/shell/command_palette.rs`: 737, 740
- `crates/ui/src/state.rs`: 6318, 6331, 6333, 6339
- `crates/ui/src/theme.rs`: 54
- `crates/ui/src/transcript.rs`: 2279, 13744, 13749
- `crates/ui/tests/brand_name.rs`: 15, 16, 17, 18, 19, 20, 21, 23, 25, 26, 27, 28, 29, 30, 33, 34, 36, 38, 41, 49, 125, 146
- `crates/update/src/lib.rs`: 425, 449, 450, 455, 473, 523, 541, 996, 1001, 1105
- `crates/update/src/windows.rs`: 92
- `dist/README.md`: 12, 39, 40, 41, 46
- `dist/macos/Info-dev.plist`: 10
- `dist/macos/Info.plist`: 12, 34
- `dist/zeron.desktop`: 6, 7, 8, 12, 13
- `docs/PARITY.md`: 46
- `docs/chat2-sync.md`: 211
- `docs/generated-images-validation.md`: 133, 134
- `docs/mcp.md`: 3, 5, 6, 64, 141
- `docs/memory-plan.md`: 9
- `docs/mobile-polish.md`: 64, 65, 83
- `docs/performance-composer.md`: 60
- `docs/performance-idle-presence.md`: 180
- `docs/performance-macos-stability.md`: 157
- `docs/performance-resource-usage.md`: 15, 20, 46, 59
- `docs/performance-runway-scroll.md`: 67
- `docs/reference/message-queue.md`: 86
- `docs/reference/windows-development.md`: 24
- `docs/registry-sync.md`: 50
- `docs/toolcraft/agent-worklog.md`: 94, 97, 154
- `edge/scripts/chat2-crosscheck.mjs`: 91
- `edge/src/install.sh`: 24, 29, 37, 44, 50, 51, 55, 63, 97, 110, 115, 118, 119, 120, 123, 124
- `scripts/dev-demo.sh`: 64, 65
- `scripts/e2e-smoke.sh`: 90, 96, 107
- `scripts/package-linux.sh`: 24, 27, 32, 43
- `scripts/package-macos.sh`: 34
- `scripts/rename-map.sh`: 14
- `scripts/run-macos-dev.sh`: 17, 26
- `scripts/run_release_tests.sh`: 122

### 2 Internal/review

- `.github/workflows/ui-tests.yml`: 165
- `.github/workflows/windows.yml`: 101, 122
- `ARCHITECTURE.md`: 1, 3, 28, 35, 36, 42, 60, 115, 125, 206, 222, 230, 234, 260, 274
- `THIRD_PARTY_NOTICES.md`: 24, 28, 35, 57, 107, 139
- `apps/mast/src/auth_cli.rs`: 1, 3, 5, 79, 124, 159, 217
- `apps/mast/src/daemon.rs`: 1, 5, 8, 17
- `apps/mast/src/main.rs`: 1, 2, 51, 172, 347
- `apps/mast/src/update_cli.rs`: 1
- `crates/doc/src/commands.rs`: 3
- `crates/doc/src/constants.rs`: 1
- `crates/doc/src/lib.rs`: 3, 7
- `crates/doc/src/parts.rs`: 354
- `crates/doc/src/schema.rs`: 1126
- `crates/doc/src/workspace.rs`: 1, 20, 22, 464
- `crates/engine/src/agent_accounts.rs`: 2, 15, 37, 167, 873, 1240
- `crates/engine/src/agent_tools.rs`: 3, 349, 361, 383, 2211
- `crates/engine/src/auth.rs`: 2
- `crates/engine/src/diff_sync.rs`: 2
- `crates/engine/src/doc_host.rs`: 4, 903, 2778, 5545
- `crates/engine/src/instance_lock.rs`: 109
- `crates/engine/src/lib.rs`: 853, 1326, 1351, 1414
- `crates/engine/src/repos.rs`: 2, 1224, 1386, 2078
- `crates/engine/src/rpc.rs`: 714
- `crates/engine/src/run_journal.rs`: 1, 412
- `crates/engine/src/sessions.rs`: 4, 67, 163, 184, 296, 746, 930, 1215, 1310, 1738, 1785, 2599, 2604, 2703
- `crates/engine/src/terminals.rs`: 2, 12
- `crates/engine/src/titles.rs`: 36, 318
- `crates/engine/src/uploads.rs`: 2, 6
- `crates/engine/src/workspace_host.rs`: 667, 1664, 1671
- `crates/engine/tests/e2e.rs`: 650, 1789
- `crates/engine/tests/m5_repos_diffs_terminals.rs`: 213
- `crates/engine/tests/restart_resume.rs`: 6, 8, 708
- `crates/engine/tests/turn_quiesce.rs`: 9
- `crates/harness/examples/antigravity_acp_probe.rs`: 3
- `crates/harness/src/acp/mod.rs`: 22, 263, 431, 456, 1451, 1626, 1631, 1953, 2413
- `crates/harness/src/acp/normalize.rs`: 165
- `crates/harness/src/agent_tools.rs`: 5, 26, 28
- `crates/harness/src/claude/catalog.rs`: 1
- `crates/harness/src/codex/catalog.rs`: 1, 57
- `crates/harness/src/codex/mod.rs`: 26, 41
- `crates/harness/src/cursor/mod.rs`: 253
- `crates/harness/src/cursor/shim.mjs`: 4, 283
- `crates/harness/src/jsonrpc.rs`: 168
- `crates/harness/src/lib.rs`: 86, 427
- `crates/harness/tests/fixtures/fake-antigravity-acp.sh`: 8
- `crates/harness/tests/fixtures/fake-cursor-shim.sh`: 2
- `crates/mcp/src/lib.rs`: 3, 5, 22, 35, 70, 71, 80
- `crates/mcp/src/mast.rs`: 102, 127
- `crates/mcp/src/tools.rs`: 767
- `crates/proto/src/agent.rs`: 146, 378, 707, 721, 722, 738
- `crates/proto/src/entities.rs`: 3, 82, 329
- `crates/proto/src/lib.rs`: 3
- `crates/proto/src/motion.rs`: 1, 18, 30, 69, 93, 94, 108
- `crates/proto/src/view.rs`: 142, 323, 353, 398, 511
- `crates/rpc/src/device_room.rs`: 16, 311, 761, 778, 805
- `crates/rpc/src/lib.rs`: 5, 125
- `crates/sync/src/chat_client.rs`: 403
- `crates/sync/src/registry.rs`: 191
- `crates/sync/src/types.rs`: 2, 49
- `crates/theme/src/lib.rs`: 289, 291
- `crates/theme/src/vscode.rs`: 65, 75
- `crates/ui/src/app_menus.rs`: 3, 22, 52, 159
- `crates/ui/src/appshots.rs`: 592, 610
- `crates/ui/src/attachments.rs`: 6, 101, 466
- `crates/ui/src/changes.rs`: 4857, 6664
- `crates/ui/src/composer.rs`: 51, 56, 61, 306, 4172, 7744, 7765, 7782, 7958, 8206, 8513, 10022, 10039
- `crates/ui/src/icons.rs`: 3, 9, 12, 93, 178, 256
- `crates/ui/src/lib.rs`: 9, 13, 444
- `crates/ui/src/loaders.rs`: 1, 67, 107
- `crates/ui/src/markdown/mend.rs`: 2, 15, 42
- `crates/ui/src/markdown/parser.rs`: 306
- `crates/ui/src/markdown/render.rs`: 30, 47, 53, 55, 57, 62, 66, 1211, 1410, 1437
- `crates/ui/src/markdown/selection.rs`: 5
- `crates/ui/src/motion.rs`: 1, 295, 401, 532, 610, 967
- `crates/ui/src/notice.rs`: 2, 21
- `crates/ui/src/overview_grouping.rs`: 278
- `crates/ui/src/pickers.rs`: 241, 258, 3799, 3817, 4413, 4449, 4848, 6211
- `crates/ui/src/popover.rs`: 302, 823, 866, 883, 914, 1079, 1094, 1110, 1163
- `crates/ui/src/settings.rs`: 2, 741, 975, 2955
- `crates/ui/src/settings/accounts.rs`: 102, 121, 302, 311, 958, 1002, 1031, 1071, 1387, 1531, 1610, 1722, 1808
- `crates/ui/src/settings/archived.rs`: 151, 165, 256, 296, 306
- `crates/ui/src/settings/composer.rs`: 25, 51, 125
- `crates/ui/src/settings/devices.rs`: 230, 287, 336
- `crates/ui/src/settings/shortcuts.rs`: 244, 281, 517, 773
- `crates/ui/src/settings/widgets.rs`: 4, 168, 406
- `crates/ui/src/shell.rs`: 1, 4, 285, 338, 551, 699, 733, 744, 1917, 2687, 4160, 4173, 4182, 6272, 6276, 6352, 6404, 6621, 6901, 6923, 7018, 7036, 7366, 8284, 8478, 10266, 11194, 11223, 11294, 11399, 11531, 11620, 11639, 11671, 11725, 11773, 11803, 11822, 12093, 12385, 12442, 13340, 13649, 14001, 14002, 14007, 14031, 14046, 14057
- `crates/ui/src/state.rs`: 4584
- `crates/ui/src/terminal/panel.rs`: 1442, 1488, 1531, 1612, 1631
- `crates/ui/src/theme.rs`: 4, 779, 818, 827, 1126, 1225, 1900, 2000, 2817
- `crates/ui/src/transcript.rs`: 1221, 3200, 5298, 7047, 7674, 7756, 7758
- `crates/ui/tests/brand_name.rs`: 2, 4
- `crates/update/src/lib.rs`: 2, 245, 490
- `edge/src/auth-routes.ts`: 2, 164
- `edge/src/install.sh`: 89, 90
- `edge/src/session-doc/tail.ts`: 2
- `scripts/acp-starve-repro.py`: 14
- `scripts/dev-demo.sh`: 24, 39
- `scripts/e2e-smoke.sh`: 8, 92
- `scripts/package-windows.ps1`: 28
- `scripts/profile-composer.py`: 19, 93
- `scripts/rename-map.sh`: 15, 16, 18, 20, 28, 34, 35
- `scripts/resource-profile.mjs`: 178
- `scripts/run-macos-dev.sh`: 57
- `scripts/test-windows-capture-client.ps1`: 14, 31, 34, 37, 38, 39, 46, 53
- `scripts/test-windows-lifecycle.ps1`: 15, 20, 33, 63, 65
- `scripts/test-windows-startup.ps1`: 32

### 3 Migration/compatibility

- `.github/workflows/deploy.yml`: 8, 69, 105
- `.github/workflows/release.yml`: 8, 9, 165, 184
- `.github/workflows/testflight.yml`: 79, 82, 104, 111, 113, 133
- `.github/workflows/ui-tests.yml`: 162
- `.github/workflows/windows.yml`: 103, 136
- `ARCHITECTURE.md`: 25, 130, 158, 159, 207, 235, 245
- `README.md`: 58
- `THIRD_PARTY_NOTICES.md`: 38
- `apps/ios/README.md`: 15, 19, 68
- `apps/ios/Zeron.xcodeproj/project.pbxproj`: 71, 107, 138, 318, 324, 344, 350, 368, 374, 386, 404, 410, 422
- `apps/ios/Zeron.xcodeproj/xcshareddata/xcschemes/Zeron.xcscheme`: 18, 20, 38, 48, 68, 70
- `apps/ios/Zeron/App/AppModel.swift`: 73, 669, 789
- `apps/ios/Zeron/App/DemoDataset.swift`: 54, 58, 60, 63, 65, 68, 76, 79, 148, 173, 273, 317
- `apps/ios/Zeron/Auth/AuthClient.swift`: 89
- `apps/ios/Zeron/Info.plist`: 29
- `apps/ios/Zeron/Models/Entities.swift`: 123, 304, 333, 346, 355
- `apps/ios/Zeron/Sync/ChatRoomClient.swift`: 23
- `apps/ios/Zeron/Views/Loaders.swift`: 100, 111
- `apps/ios/Zeron/Views/SignInView.swift`: 13, 95, 239
- `apps/ios/ZeronTests/ChangeRequestTrackingTests.swift`: 20
- `apps/ios/ZeronTests/SessionStoreDurabilityTests.swift`: 472, 510, 550, 601
- `apps/landing/downloads.test.mjs`: 19, 21, 29, 32
- `apps/landing/public/downloads.js`: 2, 19, 25
- `apps/landing/public/index.html`: 11, 780, 911, 996, 1010, 1409, 1419, 1420, 1421, 1422
- `apps/landing/public/telemetry.js`: 4, 67, 68
- `apps/landing/telemetry.test.mjs`: 11, 17, 67, 132, 136, 153, 182, 191, 200
- `apps/landing/wrangler.jsonc`: 7, 12, 13
- `apps/mast/build.rs`: 38, 39, 42
- `apps/mast/src/daemon.rs`: 15, 18, 237, 243, 253, 256, 400, 422, 425, 442, 444, 451
- `apps/mast/src/main.rs`: 80, 82, 570, 571
- `apps/mast/src/paths.rs`: 11, 33
- `apps/mast/src/update_cli.rs`: 68
- `apps/www-redirect/src/index.js`: 4
- `apps/www-redirect/wrangler.jsonc`: 2, 8, 13
- `crates/doc/src/parts.rs`: 707
- `crates/doc/src/registry/artifacts.rs`: 347
- `crates/engine/src/agent_tools.rs`: 2212, 2213, 2214
- `crates/engine/src/artifacts.rs`: 9, 40, 133, 147, 168, 270, 836, 2246, 2452, 2458, 2464, 2476, 2480, 2487
- `crates/engine/src/change_requests.rs`: 482, 493
- `crates/engine/src/chat2_host.rs`: 475, 489
- `crates/engine/src/diff_sync.rs`: 2834
- `crates/engine/src/diff_targets/refs.rs`: 155
- `crates/engine/src/doc_host.rs`: 5358
- `crates/engine/src/execution_scope/tests.rs`: 27
- `crates/engine/src/external_import.rs`: 687, 1086, 1566, 2407, 2420, 2574, 2579
- `crates/engine/src/lib.rs`: 146
- `crates/engine/src/linear/credentials.rs`: 13
- `crates/engine/src/liveness.rs`: 229, 241
- `crates/engine/src/project_actions.rs`: 1, 30, 32, 352, 356, 360, 811
- `crates/engine/src/repos.rs`: 7, 9, 52, 55, 175, 213, 1143, 1223, 1226, 1272, 2272, 2273, 2274
- `crates/engine/src/run_journal.rs`: 485
- `crates/engine/src/sessions.rs`: 1188, 3349
- `crates/engine/src/source_control.rs`: 977, 993, 1070, 1084, 1091, 1141, 1150, 1163, 1193, 1527, 1605, 1624
- `crates/engine/src/terminals.rs`: 253, 1179
- `crates/engine/src/titles.rs`: 54, 150, 764, 771, 775, 779
- `crates/engine/src/transcript_history.rs`: 10
- `crates/engine/src/uploads.rs`: 16
- `crates/engine/src/workspace_files.rs`: 918, 2434, 2522
- `crates/engine/tests/agent_tools.rs`: 166, 1281
- `crates/engine/tests/artifacts.rs`: 236, 239, 336, 396, 401, 629, 630, 673, 678, 725, 729
- `crates/engine/tests/codex_login_resolution.rs`: 63
- `crates/engine/tests/device_routing.rs`: 287, 295, 867, 904
- `crates/engine/tests/diff_targets.rs`: 20
- `crates/engine/tests/diff_targets_sync.rs`: 25
- `crates/engine/tests/external_import_auto_adopt.rs`: 102, 110, 123, 126, 134, 173, 176, 181, 183, 189
- `crates/engine/tests/external_import_junk_filter.rs`: 82, 183, 185, 186, 207, 232
- `crates/engine/tests/external_import_sync.rs`: 138
- `crates/engine/tests/external_import_title_repair_harness_session.rs`: 3, 80, 90, 141
- `crates/engine/tests/github_cli_login_shell.rs`: 54, 81
- `crates/engine/tests/instruction_delivery.rs`: 283
- `crates/engine/tests/m5_repos_diffs_terminals.rs`: 154, 178, 1504, 1617, 1618, 2057, 2058, 2089, 2163
- `crates/engine/tests/m5c_accounts_uploads_titles.rs`: 779
- `crates/engine/tests/perf_bench.rs`: 182, 183, 335
- `crates/engine/tests/pr_review_launch.rs`: 43
- `crates/engine/tests/pr_review_sync.rs`: 47
- `crates/engine/tests/project_action_commands.rs`: 20
- `crates/engine/tests/ref_diff_targets.rs`: 32, 500
- `crates/engine/tests/restart_resume.rs`: 943
- `crates/engine/tests/work_outline_execution_rpc.rs`: 43
- `crates/engine/tests/work_outline_launch.rs`: 52
- `crates/engine/tests/work_outline_pr_links.rs`: 36
- `crates/engine/tests/worktree_on_run.rs`: 4, 215, 288
- `crates/harness/examples/antigravity_acp_probe.rs`: 86
- `crates/harness/examples/cursor_stability_probe.rs`: 93
- `crates/harness/src/acp/mod.rs`: 221, 3173, 3425, 3711, 3760, 3832, 3895, 4321
- `crates/harness/src/adapter_install.rs`: 9, 62, 67, 68, 103, 375, 413
- `crates/harness/src/agent_tools.rs`: 169, 180, 220, 265, 279
- `crates/harness/src/archive_install.rs`: 6
- `crates/harness/src/claude/instructions.rs`: 4, 81, 186
- `crates/harness/src/claude/mod.rs`: 465, 1201, 1319
- `crates/harness/src/codex/mod.rs`: 261, 316, 1099, 2136, 2148
- `crates/harness/src/cursor/mod.rs`: 2, 19, 61
- `crates/harness/src/cursor/shim.mjs`: 1, 111, 148, 527
- `crates/harness/src/cursor/state.rs`: 20, 59
- `crates/harness/src/executable.rs`: 46
- `crates/harness/src/instruction_support.rs`: 302
- `crates/harness/src/process/windows/tests.rs`: 254
- `crates/harness/src/scratch.rs`: 47
- `crates/harness/src/shell_env.rs`: 50, 67, 68, 174, 310, 315, 326, 333
- `crates/harness/tests/codex.rs`: 1681
- `crates/harness/tests/cursor_shim.rs`: 405, 431
- `crates/harness/tests/fixtures/codex/live-0.146.1-multi-agent.jsonl`: 1
- `crates/harness/tests/fixtures/fake-acp.sh`: 150
- `crates/harness/tests/fixtures/fake-codex.sh`: 21, 105
- `crates/harness/tests/managed_install.rs`: 94
- `crates/harness/tests/windows_native.rs`: 104, 107, 115, 118
- `crates/mcp/src/lib.rs`: 79, 86, 87, 98, 99
- `crates/mcp/src/tools.rs`: 67, 766, 774, 806, 838, 843, 867, 868, 870, 873, 913, 930, 948, 964, 982, 983, 984, 995, 999, 1035, 1036, 1037, 1063, 1079, 1104, 1168, 1178, 1188, 1194, 1235, 1250, 1295, 1328, 1337, 1359, 1410, 1419, 1484, 1530, 1586, 1609, 1610, 1611, 1654, 1667, 1706, 1717, 1730, 1745, 1770, 1802, 1819, 1824, 1833, 1891, 1961, 1972, 1982, 2003, 2007
- `crates/proto/src/agent.rs`: 177, 725, 728, 739, 740, 921
- `crates/proto/src/artifacts.rs`: 11, 44, 50, 54, 1136, 1293, 1294, 1303
- `crates/proto/src/artifacts/comments.rs`: 5, 41, 122, 311, 432
- `crates/proto/src/artifacts/questions.rs`: 118, 380, 391
- `crates/proto/src/artifacts/tests.rs`: 17, 19, 408, 409, 410, 425, 431, 473, 474
- `crates/proto/src/entities.rs`: 552, 1582, 1646
- `crates/proto/src/env.rs`: 1, 2, 13, 15, 22, 34, 48, 56, 71
- `crates/proto/src/preview.rs`: 23, 25, 69, 72, 78, 84
- `crates/proto/src/view.rs`: 564
- `crates/proto/src/work_outline_instructions.rs`: 751, 753, 1005, 1009, 1031, 1040
- `crates/proto/src/work_outline_instructions/tests.rs`: 332, 349, 363, 364, 388, 389, 651, 658, 1340, 1346, 1350, 1359
- `crates/rpc/src/lib.rs`: 517
- `crates/sync/examples/transport_live.rs`: 210
- `crates/sync/src/net_path.rs`: 70
- `crates/theme/src/builtins.rs`: 15, 239, 273
- `crates/theme/src/lib.rs`: 271, 276, 277, 832, 836, 837
- `crates/ui/build.rs`: 8
- `crates/ui/examples/browser-fixture.rs`: 129
- `crates/ui/src/appshots/linux/mod.rs`: 161, 162
- `crates/ui/src/artifacts/comments.rs`: 199, 201, 212, 230, 286, 304, 306, 432, 434
- `crates/ui/src/artifacts/links.rs`: 7, 64, 65, 192, 200, 201, 204, 263
- `crates/ui/src/artifacts/model.rs`: 11, 299, 314
- `crates/ui/src/artifacts/question_html.rs`: 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 60, 62, 63, 282, 346, 348, 349, 352, 357
- `crates/ui/src/artifacts/tab.rs`: 448
- `crates/ui/src/artifacts/tests.rs`: 22, 112, 113, 122, 130, 132, 137, 563
- `crates/ui/src/browser/document_view.rs`: 18, 29, 230, 310, 316, 326
- `crates/ui/src/browser/linux/mod.rs`: 55, 61
- `crates/ui/src/browser/macos.rs`: 1127
- `crates/ui/src/browser/model.rs`: 166
- `crates/ui/src/build_watch/tests.rs`: 38
- `crates/ui/src/change_requests.rs`: 450
- `crates/ui/src/changes.rs`: 8222
- `crates/ui/src/chat_closeout.rs`: 314, 472, 490, 515, 708, 714, 720, 756, 771, 789, 798, 886, 887, 894, 895
- `crates/ui/src/composer.rs`: 7149, 9922
- `crates/ui/src/links.rs`: 44, 46, 54, 157, 158, 171, 172
- `crates/ui/src/loaders.rs`: 23, 56
- `crates/ui/src/markdown/render.rs`: 32, 1166
- `crates/ui/src/motion.rs`: 10, 584
- `crates/ui/src/notify.rs`: 77
- `crates/ui/src/overview.rs`: 7082, 7084, 7085, 7086, 7087, 7088, 7089, 7090, 7091, 7092, 7124, 7125, 7126, 7127, 7468, 7469, 7505, 7548, 7817, 7841, 8263, 9097
- `crates/ui/src/overview_grouping.rs`: 279
- `crates/ui/src/pickers.rs`: 136, 639, 7004, 7011, 7028, 7048, 7055, 7063
- `crates/ui/src/popover.rs`: 1215
- `crates/ui/src/settings.rs`: 2394
- `crates/ui/src/settings/accounts.rs`: 167, 1157, 1211, 1313, 1514, 1764
- `crates/ui/src/settings/archived.rs`: 232
- `crates/ui/src/settings/devices.rs`: 395
- `crates/ui/src/settings/shortcuts.rs`: 71
- `crates/ui/src/shell.rs`: 4429, 6682, 9343, 9364, 9367, 13339
- `crates/ui/src/shell/artifacts_ui.rs`: 466, 718
- `crates/ui/src/sound.rs`: 206
- `crates/ui/src/state.rs`: 7, 94, 5716, 6319, 6340, 6341, 6346, 6371, 6374
- `crates/ui/src/theme.rs`: 2001
- `crates/ui/src/workspace_chip.rs`: 436, 438, 485, 490
- `crates/ui/src/workspace_links.rs`: 9, 257, 260
- `crates/ui/tests/brand_name.rs`: 14, 24, 31, 45, 145
- `crates/update/src/lib.rs`: 12, 104, 107, 110, 113, 192, 244, 316, 323, 500, 503, 511, 522, 542, 976, 981, 986, 991, 1012, 1015, 1031, 1068, 1084, 1089
- `crates/update/src/windows.rs`: 18, 29, 44, 75, 133, 293, 295, 304, 314, 329, 364, 383, 435
- `dist/README.md`: 10, 13, 14, 23, 28, 30
- `dist/brand/build-assets.sh`: 13
- `dist/macos/Info-dev.plist`: 14
- `dist/macos/Info.plist`: 16, 31
- `dist/windows/README.md`: 3, 7
- `dist/windows/zeron.rc`: 2
- `docs/chat2-sync.md`: 146
- `docs/generated-images-validation.md`: 131
- `docs/mcp.md`: 21, 24, 25
- `docs/memory-plan.md`: 11, 116
- `docs/mobile-polish.md`: 60
- `docs/perf-baseline.md`: 29, 34, 35, 36
- `docs/performance-macos.md`: 144, 151, 158, 159
- `docs/performance-resource-usage.md`: 32, 196
- `docs/performance/resource-baseline.json`: 6, 80
- `docs/performance/resource-candidate.json`: 6, 92
- `docs/performance/resource-four-workers.json`: 7, 93, 184, 270, 361, 447
- `docs/performance/resource-live.json`: 3
- `docs/performance/resource-long-baseline.json`: 3
- `docs/performance/resource-long-candidate.json`: 3
- `docs/performance/runway-layout.json`: 87, 177, 267, 357, 447, 537, 627, 717
- `docs/performance/runway-scroll.json`: 4, 5, 17, 106, 195, 284, 375, 464, 553, 642
- `docs/planning-artifact-conventions.md`: 42, 43, 44
- `docs/reference/project-actions.md`: 9, 13, 15, 49, 59
- `docs/reference/windows-development.md`: 5
- `docs/theme-system.md`: 130
- `docs/toolcraft/agent-worklog.md`: 95
- `docs/transcript-browser-links.md`: 28, 40
- `docs/transport-reliability.md`: 120, 127, 143
- `docs/transport-results/2026-09-15/metadata.json`: 6
- `edge/package-lock.json`: 2, 8
- `edge/package.json`: 2
- `edge/src/env.ts`: 30, 36
- `edge/src/index.ts`: 122
- `edge/src/install.sh`: 4, 7, 10, 12, 13, 16, 25, 45, 46, 72, 80, 83
- `edge/src/preview-room.test.ts`: 3
- `edge/src/preview-room.ts`: 19, 22
- `edge/src/session-doc/control-types.ts`: 2
- `edge/src/session-doc/index.ts`: 2
- `edge/test/workerd/preview.workerd.test.ts`: 10
- `edge/wrangler.jsonc`: 18, 20, 21, 23, 24, 25, 27, 28, 29
- `edge/wrangler.transport-test.jsonc`: 4, 59, 63
- `scripts/dev-demo.sh`: 8, 12, 13, 56, 57, 58, 59
- `scripts/e2e-smoke.sh`: 26
- `scripts/package-linux.sh`: 3, 18, 33, 34, 44, 45
- `scripts/package-macos.sh`: 3, 4, 6, 9, 25, 26, 27, 41, 77, 85
- `scripts/package-windows.ps1`: 14, 35, 37, 44
- `scripts/rename-map.sh`: 17, 19, 29
- `scripts/resource-profile.mjs`: 31, 166
- `scripts/run-macos-browser-fixture.sh`: 7, 13
- `scripts/run-macos-dev.sh`: 50, 52, 56
- `scripts/test-cursor-sdk-auth.mjs`: 25
- `scripts/test-linux-browser.sh`: 79
- `scripts/test-windows-lifecycle.ps1`: 4, 80
- `scripts/test-windows-startup.ps1`: 3

### 4 Keep/history

- `.github/workflows/ui-tests.yml`: 87
- `ARCHITECTURE.md`: 10, 296
- `Cargo.lock`: 3365
- `Cargo.toml`: 46, 71, 72, 78, 79, 162
- `README.md`: 117
- `THIRD_PARTY_NOTICES.md`: 3, 16, 20
- `apps/ios/Zeron/App/DemoDataset.swift`: 96, 101, 106
- `apps/landing/public/index.html`: 787, 938, 1436, 1448, 1461
- `crates/engine/src/pr_ticket_cache.rs`: 1725
- `crates/engine/tests/external_import_titles.rs`: 184
- `crates/engine/tests/m5c_accounts_uploads_titles.rs`: 610
- `crates/harness/src/adapter_install.rs`: 7
- `crates/harness/src/claude/mod.rs`: 14
- `crates/harness/src/codex/mod.rs`: 1754
- `crates/harness/src/cursor/shim.mjs`: 492
- `crates/harness/tests/managed_install.rs`: 5
- `crates/harness/tests/managed_install_failure.rs`: 1
- `crates/theme/src/builtins.rs`: 266, 300
- `crates/ui/examples/sidebar-fixture.rs`: 75
- `crates/ui/src/markdown/parser.rs`: 1089, 1091, 1092
- `crates/ui/src/notice.rs`: 28
- `crates/ui/src/settings.rs`: 688
- `crates/ui/src/settings/accounts.rs`: 186
- `crates/ui/src/settings/composer.rs`: 2
- `crates/ui/src/settings/shortcuts.rs`: 1132
- `crates/ui/src/shell.rs`: 634, 687, 13419
- `crates/ui/src/theme.rs`: 51, 1960
- `crates/ui/src/transcript.rs`: 7679
- `crates/ui/tests/brand_name.rs`: 32
- `docs/macos-frame-recovery.md`: 23
- `docs/performance-idle-presence.md`: 75
- `docs/performance-resource-usage.md`: 8
- `docs/plans/2026-08-20-appshots-macos.md`: 9, 21, 50, 51, 52
- `docs/plans/2026-08-20-appshots-ux-refinement.md`: 16, 17, 31, 32
- `docs/plans/2026-08-22-appshots-windows-linux.md`: 15, 23, 43, 44
- `docs/plans/2026-10-05-zeron-unfinished-work-handoff.md`: 1, 7, 16, 22, 108, 218, 228, 229, 230, 231, 232, 233, 234, 235, 236, 242, 244, 250, 257, 276
- `docs/plans/artifacts-contract.md`: 18, 26, 68, 80, 95, 98, 141, 142, 145, 153, 154, 164, 165, 167, 173, 186, 193, 234
- `docs/plans/planning-artifacts-design.md`: 7, 13, 14, 15, 22, 23, 25, 30, 33, 36, 39, 48, 69, 82, 108, 126, 140, 149, 160, 168, 188, 189, 194, 207, 218, 220, 222, 231, 234
- `docs/plans/rename-migration.md`: 1, 3, 14, 15, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 35, 36, 37, 38, 39, 41, 45, 46, 47, 50, 54, 56, 57, 62, 63, 71, 74, 75, 86, 88, 94, 99, 101, 107, 111, 117, 118, 120, 121, 126, 130, 135, 137, 140, 144, 147, 158, 161, 162, 163, 167, 169, 172, 176, 178, 180, 183, 184, 186, 189, 194, 199, 201, 207, 208, 228, 234, 235, 236, 237, 241, 242
- `docs/plans/repo-scoped-collaboration-handoff.md`: 9, 47, 174, 180, 182, 264, 285, 291, 292, 293, 294, 295
- `docs/plans/work-outline-implementation-specification.md`: 1, 5, 9, 13, 58, 74, 85, 101, 114, 118, 149, 199, 210, 215, 239, 264, 303, 343, 346, 348, 369, 379, 402, 404, 410, 412, 457, 465, 520, 556, 568, 573, 588, 590, 615, 626, 680, 681, 714, 727, 789, 790, 791
- `docs/plans/work-outline/HANDOFF.md`: 7, 8, 9, 11, 15, 21, 22, 23, 36, 47, 59, 64, 76, 153, 155, 168, 237, 256, 257, 264, 265, 266, 267, 268, 270, 272, 274, 276, 277, 284, 304, 388, 399, 402, 417, 443, 470, 472, 473, 474, 476, 477, 478, 479, 638, 666, 670, 676, 679, 684, 704, 711, 773, 803, 804, 812, 827, 828, 829, 830, 831, 842, 849, 852
- `docs/plans/work-outline/linear-contract.md`: 42, 82
- `docs/plans/work-outline/m0-contract.md`: 105, 212, 222, 343, 350, 569, 570, 600, 675, 799, 861, 868, 887, 943, 982, 1107, 1119, 1123, 1124, 1125, 1128, 1130, 1131, 1133, 1135, 1153, 1169, 1170, 1184, 1290, 1294, 1340, 1385, 1386, 1388, 1398, 1478, 1544, 1603, 1619, 1624
- `docs/plans/work-outline/m0-execution-scope.md`: 44, 45, 49, 154, 197, 230, 276, 283, 284
- `docs/plans/work-outline/m0-idempotency-command-plane.md`: 4, 12, 72, 91, 106, 107, 115
- `docs/plans/work-outline/m0-instruction-delivery.md`: 5, 16, 28, 30, 59, 95, 104, 115, 127, 134, 151, 164, 233, 242, 248, 335, 347, 367, 374, 381, 387, 411
- `docs/plans/work-outline/m0-registry-persistence.md`: 3, 259
- `docs/plans/work-outline/m0-ui-integration.md`: 13, 126, 142, 144
- `docs/plans/work-outline/parked-proposals.md`: 10
- `docs/plans/work-outline/pr-page-audit.md`: 1, 3, 4, 39, 180, 184, 255
- `docs/plans/work-outline/pr-sync-contract.md`: 49
- `docs/plans/work-outline/release-testing.md`: 23
- `docs/reference/windows-development.md`: 54
- `docs/regressions/sidebar-pin-integrity.md`: 39
- `docs/research/acp.md`: 9, 14, 24, 46, 47, 61, 64, 126, 153, 158, 159, 160, 181, 193, 194, 215
- `docs/research/appshots.md`: 1, 10, 15, 17, 23, 24, 55, 71, 93, 122, 141, 147, 210, 220, 222, 224, 325, 341, 353, 396, 417, 467, 490
- `docs/research/codex-subagents.md`: 27, 42, 43, 74
- `docs/research/codex-visualize.md`: 3, 17, 50, 134, 141, 152, 156, 173, 175, 176, 185, 194
- `docs/research/cursor-auth-incident/README.md`: 76, 77, 79, 104, 125, 131, 132, 133
- `docs/research/cursor-auth-incident/results.json`: 78
- `docs/research/cursor-stability/README.md`: 13, 37, 38, 70, 101, 107, 108, 109, 115, 116, 123, 125, 126, 127, 128, 136, 137, 148
- `docs/research/cursor-stability/results.json`: 184, 189
- `docs/research/cursor-steering-stability/README.md`: 15, 35, 88, 89, 90, 92, 93, 95, 96, 97, 98, 99, 100, 108
- `docs/research/diri-sidebar-browser-plan.md`: 1, 5, 53, 54, 69, 97, 114, 118, 144, 146, 160, 162
- `docs/research/durable-objects-language.md`: 3, 28
- `docs/research/feature-inventory.md`: 1, 3, 15, 22, 27, 78, 117, 177
- `docs/research/harness.md`: 7, 33
- `docs/research/loro-rust.md`: 22, 34, 46, 57
- `docs/research/model-catalog-validation.md`: 31, 37, 38, 39, 40, 42, 60
- `docs/research/model-discovery-reliability.md`: 51, 53, 56
- `docs/research/mugen-pretext.md`: 3
- `docs/research/sidebar-view-options.md`: 12, 199, 212, 224, 229, 232, 242, 243, 265, 266, 307, 308, 309, 310, 311, 314, 319
- `docs/research/windows-click-drag-audit.md`: 19, 27, 28, 34, 35, 41, 42, 48, 49, 55, 61, 62, 63, 64, 74
- `docs/screenshots/browser/README.md`: 18
- `scripts/rename-map.sh`: 2

## Tracked paths containing either name

These include the iOS directory tree; names alone do not imply user exposure.

- `apps/ios/Zeron.xcodeproj/project.pbxproj`
- `apps/ios/Zeron.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved`
- `apps/ios/Zeron.xcodeproj/xcshareddata/xcschemes/Zeron.xcscheme`
- `apps/ios/Zeron/App/AppConfig.swift`
- `apps/ios/Zeron/App/AppModel.swift`
- `apps/ios/Zeron/App/BenchRunner.swift`
- `apps/ios/Zeron/App/DemoDataset.swift`
- `apps/ios/Zeron/App/E2ERunner.swift`
- `apps/ios/Zeron/App/ZeronApp.swift`
- `apps/ios/Zeron/Assets.xcassets/AccentColor.colorset/Contents.json`
- `apps/ios/Zeron/Assets.xcassets/AppIcon.appiconset/AppIcon1024.png`
- `apps/ios/Zeron/Assets.xcassets/AppIcon.appiconset/Contents.json`
- `apps/ios/Zeron/Assets.xcassets/Contents.json`
- `apps/ios/Zeron/Assets.xcassets/LaunchBackground.colorset/Contents.json`
- `apps/ios/Zeron/Auth/AuthClient.swift`
- `apps/ios/Zeron/Composer/Appshots.swift`
- `apps/ios/Zeron/Composer/Attachments.swift`
- `apps/ios/Zeron/Composer/ComposerEditor.swift`
- `apps/ios/Zeron/Composer/ComposerView.swift`
- `apps/ios/Zeron/Composer/QueuePanelView.swift`
- `apps/ios/Zeron/Fonts/Geist-Bold.ttf`
- `apps/ios/Zeron/Fonts/Geist-Medium.ttf`
- `apps/ios/Zeron/Fonts/Geist-SemiBold.ttf`
- `apps/ios/Zeron/Fonts/Geist.ttf`
- `apps/ios/Zeron/Fonts/GeistMono.ttf`
- `apps/ios/Zeron/Info.plist`
- `apps/ios/Zeron/Markdown/Highlight.swift`
- `apps/ios/Zeron/Markdown/InlineCodeRenderer.swift`
- `apps/ios/Zeron/Markdown/MarkdownBlockView.swift`
- `apps/ios/Zeron/Markdown/MarkdownModel.swift`
- `apps/ios/Zeron/Models/Entities.swift`
- `apps/ios/Zeron/Models/HarnessCatalog.swift`
- `apps/ios/Zeron/Models/MessageQueue.swift`
- `apps/ios/Zeron/Models/NewSessionDestination.swift`
- `apps/ios/Zeron/Sync/ChangeRequestTracker.swift`
- `apps/ios/Zeron/Sync/ChatFrames.swift`
- `apps/ios/Zeron/Sync/ChatRoomClient.swift`
- `apps/ios/Zeron/Sync/Connectivity.swift`
- `apps/ios/Zeron/Sync/DeviceRelayClient.swift`
- `apps/ios/Zeron/Sync/DocDisk.swift`
- `apps/ios/Zeron/Sync/LoroValueJSON.swift`
- `apps/ios/Zeron/Sync/OnlineBus.swift`
- `apps/ios/Zeron/Sync/RegistryClient.swift`
- `apps/ios/Zeron/Sync/RegistryCore.swift`
- `apps/ios/Zeron/Sync/SessionQueue.swift`
- `apps/ios/Zeron/Sync/SessionStore.swift`
- `apps/ios/Zeron/Sync/TransportErrorDescription.swift`
- `apps/ios/Zeron/Sync/UploadStash.swift`
- `apps/ios/Zeron/Sync/WorkspaceStore.swift`
- `apps/ios/Zeron/Theme/BrandMarks.swift`
- `apps/ios/Zeron/Theme/HermesMark.swift`
- `apps/ios/Zeron/Theme/LineIcons.swift`
- `apps/ios/Zeron/Theme/Motion.swift`
- `apps/ios/Zeron/Theme/Theme.swift`
- `apps/ios/Zeron/Transcript/NativeTranscriptTable.swift`
- `apps/ios/Zeron/Transcript/TranscriptLayoutProbe.swift`
- `apps/ios/Zeron/Transcript/TranscriptRows.swift`
- `apps/ios/Zeron/Transcript/TranscriptView.swift`
- `apps/ios/Zeron/Transcript/Veil.swift`
- `apps/ios/Zeron/Views/ArchivedShelf.swift`
- `apps/ios/Zeron/Views/HomeView.swift`
- `apps/ios/Zeron/Views/Loaders.swift`
- `apps/ios/Zeron/Views/NewSessionView.swift`
- `apps/ios/Zeron/Views/PullRequestBadge.swift`
- `apps/ios/Zeron/Views/SessionHostPickerSheet.swift`
- `apps/ios/Zeron/Views/SessionView.swift`
- `apps/ios/Zeron/Views/SheetUI.swift`
- `apps/ios/Zeron/Views/SignInView.swift`
- `apps/ios/Zeron/Views/SpaceView.swift`
- `apps/ios/ZeronTests/AppshotTests.swift`
- `apps/ios/ZeronTests/ChangeRequestTrackingTests.swift`
- `apps/ios/ZeronTests/ChatFramesTests.swift`
- `apps/ios/ZeronTests/ComposerEditorTests.swift`
- `apps/ios/ZeronTests/DeviceRelayClientTests.swift`
- `apps/ios/ZeronTests/GeneratedImageTests.swift`
- `apps/ios/ZeronTests/HarnessCatalogTests.swift`
- `apps/ios/ZeronTests/MessageQueueTests.swift`
- `apps/ios/ZeronTests/NetworkReliabilityTests.swift`
- `apps/ios/ZeronTests/PinnedSessionsTests.swift`
- `apps/ios/ZeronTests/ProjectlessSessionTests.swift`
- `apps/ios/ZeronTests/RegistryCoreTests.swift`
- `apps/ios/ZeronTests/RegistryDocTests.swift`
- `apps/ios/ZeronTests/SessionStoreDurabilityTests.swift`
- `apps/ios/ZeronTests/SessionStoreLifecycleTests.swift`
- `apps/ios/ZeronTests/StoreResidencyTests.swift`
- `apps/ios/ZeronTests/TranscriptFollowTests.swift`
- `apps/ios/ZeronTests/TranscriptLayoutTests.swift`
- `apps/ios/ZeronTests/TranscriptPresentationTests.swift`
- `apps/ios/ZeronTests/TranscriptScrollMatrixTests.swift`
- `apps/ios/ZeronTests/TransportErrorDescriptionTests.swift`
- `apps/ios/ZeronUITests/AppshotUITests.swift`
- `apps/ios/ZeronUITests/MobilePolishTests.swift`
- `apps/ios/ZeronUITests/ProjectlessSessionUITests.swift`
- `apps/landing/public/assets/icons/zeron-logo.svg`
- `apps/landing/public/assets/zeron-app-icon.png`
- `apps/landing/public/assets/zeron-favicon-v3.png`
- `dist/windows/zeron.ico`
- `dist/windows/zeron.rc`
- `dist/zeron.desktop`
- `dist/zeron.png`
- `docs/plans/2026-10-05-zeron-unfinished-work-handoff.md`
