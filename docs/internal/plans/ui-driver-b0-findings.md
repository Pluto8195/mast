# B0 spike findings: headless UI driver (S1-S3)

Throwaway spike, branch `spike/ui-driver-b0`, not for merge. Test: `crates/ui/tests/spike_b0.rs`
(`cargo test -p mast-ui --features snapshot --test spike_b0 -- --ignored --nocapture`).
Fork paths are under the zui checkout, rev 53869c2, `crates/gpui/src/`.

## S1: injected click + key, PNG, inside sandbox-exec: YES
- Real `Shell` (`snapshot::chat_fixture`) on `HeadlessAppContext` (`app/headless_app_context.rs:65`, `:104`) renders a 2400x1600 PNG via `capture_screenshot` (`:168`).
- The headless context has no input API, but `Window::dispatch_keystroke` (`window.rs:5000`) and `Window::dispatch_event` (`:5042`) are public and reachable through `update_window` (`:155`). No fork change needed.
- Typing "hello driver" reached the composer (visible in `c-clicked.png`; 2413 px changed). MouseMove/Down/Up at a fixed point changed 1028 px (hover/selection). Both counts identical across runs, so deterministic.
- Same test binary under `scripts/soak/sandbox.sb.in` (`sandbox-exec`, `env -i`, scratch HOME/TMPDIR): passes, same diffs, PNGs written. Metal offscreen works; no permission prompt, no window, no Screen Recording/Accessibility.
- Not proven: the click used a hard-coded point, so it shows event delivery, not a locator-driven action (needs S3's bounds enumeration).

## S2: a11y tree + settle: NO today, small gap; settle: YES
- `Window::debug_a11y_tree_json` (`window.rs:6052`) returns `None`; `is_a11y_active()` is false on the test platform. Cause: the tree is built only when the platform calls the `A11yCallbacks.activation` closure (`window.rs:1437-1470`), and `Platform::a11y_init` defaults to a no-op (`platform.rs:953`), TestWindow does not override it.
- Fix (not built here): override `a11y_init` on the test window to store the callbacks and call `activation` (fork patch, `cfg(test-support)`). Callbacks type is public (`platform.rs:706`). Estimated a few dozen lines. Whether the tree carries useful Mast roles/labels is unverified until then.
- Settle: `App::set_reduce_motion(true)` (`app.rs:1016`) makes `with_animation` jump to its end state (`elements/animation.rs:158`+). Verified: PNG byte-identical to the 800 ms-sleep path (`snapshot.rs:56`), 1.3 s to 0.5 s elapsed. No fork change. Unverified: UI code with its own `Instant` animations (31 `with_animation` uses; `transcript.rs:2638` already takes a reduce_motion arg).

## S3: debug-bounds enumeration + clock: bounds NO (small gap); clock reads counted
- `debug_bounds` is populated under `cfg(any(feature = "test-support", test))` (`elements/div.rs:2320`) in `rendered_frame.debug_bounds` (`window.rs:893`, `pub(crate)`). Only lookup-by-selector exists, on `VisualTestContext` (`app/test_context.rs:888`); `HeadlessAppContext` has none. Fix: a `pub` accessor that returns the map (fork patch, ~10 lines).
- Clock: `TestDispatcher` time is simulated (`advance_clock`, `platform/test/dispatcher.rs:56`) but GPUI and Mast read `std::time::Instant::now()` directly (`animation.rs:158`, `window.rs:1157`). In `crates/ui/src` (excluding `*_test(s).rs`): 231 `Instant::now()`, 195 `SystemTime/Utc/Local::now`, 130 timer/sleep/interval sites. A fixed clock for the UI would need a shim or a per-test allowlist; asserting on timing counters is possible, determinism of time-based text (e.g. "2mo", "1y" ages) is not free.

## Recommendation for B3/B4
- B3 (driver core): build on `HeadlessAppContext` + public `dispatch_*`; one fork patch (a11y activation + bounds accessor, both `cfg(test-support)`), pinned by bumping the `gaelcado/zui` rev in `Cargo.toml:169`. Containment check for B1: confirm the installed binary's gpui code is byte-equivalent without the feature, since the rev bump touches `Cargo.lock`.
- Use `set_reduce_motion(true)` for settle in tests; keep the sleep only for the PNG-only warm path.
- B4 (scenarios/determinism): treat the wall-clock reads as the long pole. Scope a clock shim to the code paths a scenario touches, plus relative-time text fixtures; do not try to virtualize all 556 sites.
- Unverified: the patched fork build, tree quality, `Instant` animations outside `with_animation`, and a locator-driven click.
