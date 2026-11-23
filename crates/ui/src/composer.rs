//! The composer: a hand-rolled multiline text input (adapted from gpui's
//! `examples/input.rs`), the compact↔expanded flip, the Send/Queue/Stop morph,
//! optimistic send with failure recovery, per-chat drafts, and the question
//! wizard that replaces the composer while a run awaits input.
//!
//! Pure decision logic (flip, auto-grow math, button morph, wizard reducer,
//! pending-input detection) lives in free functions/structs with unit tests;
//! the gpui element only feeds them measurements.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyTooltip, App, BorderStyle, Bounds, ClipboardEntry, ClipboardItem, Context, CursorStyle,
    DispatchPhase, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, KeyBinding, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, PaintQuad, PathPromptOptions, Pixels, Point, Role,
    ScrollWheelEvent, SharedString, Style, StyledImage as _, Subscription, Task, TextRun,
    TextStyle, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill, img, point,
    prelude::*, px, quad, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use mast_doc::{MessagePart, MessageRole, SessionCommandPayload, SessionMessageEntry};
use mast_proto::{
    FileSearchMatch, HarnessId, RunRequest, SandboxLevel, SlashCommand, UserInputAnswer,
    UserInputQuestion, capabilities,
};
use mast_rpc::{RpcError, methods};

use crate::appshots::{self, CapturedAppshot};

mod attachment_staging;
mod send;
mod code_ref_chips;
mod file_mention_flow;
mod file_mention_model;
pub use file_mention_model::{SentMentionSpan, sent_mention_display};
use file_mention_model::*;
mod flip_morph;
mod input;
mod input_layout_geometry;
mod input_key_bindings;
pub use input_key_bindings::init;
pub(crate) use input_key_bindings::PALETTE_SEARCH_CONTEXT;
#[cfg(test)]
use input_key_bindings::WORKSPACE_SEARCH_CONTEXT;
use input_key_bindings::*;
pub use input_layout_geometry::*;
use input_layout_geometry::{input_max_scroll, input_overflow_edges, input_drag_position, input_reveal_height, input_scroll_offset, input_scroll_offset_for_cursor, input_drag_scroll_delta, press_intent, PressIntent};
mod question_wizard;
pub use question_wizard::*;
pub use input::ComposerInput;
pub use flip_morph::*;
use flip_morph::model_handoff;
pub(crate) mod instruction_reset;
pub(crate) mod skill_completion;
pub(crate) mod skill_files;
mod slash_command_flow;
mod slash_command_model;
use slash_command_model::*;
use crate::attachments::{self, StagedAttachment};
use crate::motion;
use crate::notice::{NoticeChipIcon, notice_chip};
use crate::pickers::Pickers;
use crate::settings::{ComposerSendBehavior, platform_combo};
use crate::state::{AppState, Indicator};
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Constants + pure decision logic
// ---------------------------------------------------------------------------

/// Expanded-mode textarea vertical padding: `pt-4 pb-1` (zeron composer.tsx
/// line 578) = 16 + 4.
pub const TEXTAREA_PAD_V: f32 = 20.0;
/// The expanded textarea BOX (content + padding) is clamped by the original's
/// auto-grow effect: `ta.style.height = Math.min(Math.max(scrollHeight, 76),
/// 260)` (zeron composer.tsx line 235). The 76px floor applies even when
/// empty — it's what makes the always-expanded new-chat composer tall.
pub const TEXTAREA_MIN: f32 = 76.0;
pub const TEXTAREA_MAX: f32 = 260.0;
/// Expanded actions row: `pt-1` (4) + h-8 picker chips (32 — the tallest
/// children; composer/styles.tsx pickerChip) + `pb-2.5` (10) — zeron
/// composer-actions.tsx line 60.
pub const ACTIONS_ROW_HEIGHT: f32 = 46.0;
/// The pill's 1px hairline, top + bottom (`rounded-[26px] border`).
pub const PILL_BORDER_V: f32 = 2.0;
/// Corner radius shared by the composer and the queue tray behind it.
pub(crate) const COMPOSER_RADIUS: f32 = 26.0;
/// Expanded composer bounds, border-box: 76 + 46 + 2 = 124 when empty (the
/// new-chat canvas), 260 + 46 + 2 = 308 at the content cap.
pub const COMPOSER_MIN_HEIGHT: f32 = TEXTAREA_MIN + ACTIONS_ROW_HEIGHT + PILL_BORDER_V;
pub const COMPOSER_MAX_HEIGHT: f32 = TEXTAREA_MAX + ACTIONS_ROW_HEIGHT + PILL_BORDER_V;
/// Compact pill, border-box: one-line textarea `py-3` (24) + one 22.75px line
/// (scrollHeight rounds to 47 in the original) + the 2px hairline = 49. The
/// compact cluster (`py-1.5` + h-8 = 44) is shorter, so the textarea wins.
pub const COMPACT_TOTAL_HEIGHT: f32 = 49.0;
/// `max-w-3xl`: stable outer width of the centered composer column.
pub const COMPOSER_MAX_WIDTH: f32 = 768.0;
/// The queue reads as a narrower tray emerging from behind the composer.
const QUEUE_SIDE_INSET: f32 = 16.0;
/// The composer covers the tray's lower padding so the queue reads as emerging
/// from behind it instead of as a separate rounded pill.
pub(crate) const QUEUE_COMPOSER_OVERLAP: f32 = 18.0;
/// The original floating selector rows use the same 20px chip height as the
/// established-thread footer. Their surrounding rows own no plate or border.
const NEW_THREAD_SELECTOR_ROW_HEIGHT: f32 = 20.0;
// Accommodate the 24px usage indicator and PR badge without overflowing the
// row's equal 8px top/bottom gutters.
const SESSION_FOOTER_HEIGHT: f32 = 24.0;

/// Route chrome dissolves around the middle of the shared-element move. The
/// two ramps never overlap, which avoids duplicate picker ids/popovers while
/// still letting their surrounding geometry collapse continuously.
fn route_chrome_opacities(new_thread_chrome: f32) -> (f32, f32) {
    let new_thread = ((new_thread_chrome.clamp(0.0, 1.0) - 0.5) * 2.0).clamp(0.0, 1.0);
    let session = (((1.0 - new_thread_chrome.clamp(0.0, 1.0)) - 0.5) * 2.0).clamp(0.0, 1.0);
    (new_thread, session)
}
/// Ignore subpixel noise when the shell reports the conversation width.
const COMPOSER_WIDTH_EPSILON: f32 = 0.5;
/// Below this pill input width the composer always expands.
pub const MIN_COMPACT_INPUT_WIDTH: f32 = 200.0;
/// Input text metrics: `text-[14px] leading-relaxed` = 14 × 1.625 = 22.75.
pub const INPUT_LINE_HEIGHT: f32 = 22.75;
pub const INPUT_TEXT_SIZE: f32 = 14.0;
/// A compact ramp; the glyph-ascent inset keeps the clip edge invisible.
const INPUT_FADE_BAND: f32 = 12.0;
/// Single-select questions auto-advance after this long.
pub const AUTO_ADVANCE_MS: u64 = 220;
/// Drag-selection autoscroll runs at the display-friendly 60fps cadence.
pub const DRAG_SCROLL_FRAME_MS: u64 = 16;

/// Hysteresis slack for the expanded→compact flip: once expanded, the composer
/// only collapses when the text is comfortably narrower than the compact
/// capacity — expanding and collapsing share no boundary, so a width right at
/// the flip threshold can't oscillate between the two layouts.
pub const COLLAPSE_HYSTERESIS: f32 = 32.0;
/// During an interactive resize, collapsing back to the compact mode waits
/// until the measured widths have been stable this long. Expansion remains
/// immediate so a narrowing panel never traps the controls in a compact row.
pub const RESIZE_SETTLE_MS: u64 = 150;

/// Compact↔expanded flip with hysteresis. `capacity` is the *compact-mode*
/// input capacity (a layout-stable width: measured while compact, tracked by
/// container-width deltas while expanded — never the post-flip measured width,
/// which differs per mode and would feed back into the decision):
/// - a newline always expands;
/// - while `resizing`, an expanded composer stays expanded until sizes settle;
/// - a too-narrow pill (`capacity < MIN_COMPACT_INPUT_WIDTH`) always expands;
/// - compact expands only when `text_width > capacity`; expanded collapses
///   only when `text_width < capacity - COLLAPSE_HYSTERESIS`.
pub fn composer_flip(
    expanded: bool,
    text_width: f32,
    capacity: f32,
    has_newline: bool,
    resizing: bool,
) -> bool {
    if has_newline {
        return true;
    }
    if capacity < MIN_COMPACT_INPUT_WIDTH {
        return true;
    }
    if expanded {
        resizing || text_width >= capacity - COLLAPSE_HYSTERESIS
    } else {
        text_width > capacity
    }
}

fn composer_width_changed(previous: Option<f32>, current: f32) -> bool {
    previous.is_none_or(|previous| (current - previous).abs() > COMPOSER_WIDTH_EPSILON)
}

/// Staged-attachment strip metrics (zeron attachment-ui.tsx AttachmentStrip:
/// `flex flex-wrap gap-2 px-4 pt-3`, `size-14` thumbs).
pub const STRIP_THUMB: f32 = 56.0;
pub const STRIP_GAP: f32 = 8.0;
pub const STRIP_PAD_TOP: f32 = 12.0;
pub const STRIP_PAD_X: f32 = 16.0;

/// Height the wrap strip adds to the pill for `count` staged thumbnails at an
/// `inner_width` pill content width (0 when empty). Mirrors flex-wrap: as many
/// 56px thumbs per row as fit with 8px gaps inside the 16px side insets.
pub fn attachment_strip_height(count: usize, inner_width: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let usable = (inner_width - 2.0 * STRIP_PAD_X).max(STRIP_THUMB);
    let per_row = (((usable + STRIP_GAP) / (STRIP_THUMB + STRIP_GAP)).floor() as usize).max(1);
    let rows = count.div_ceil(per_row);
    STRIP_PAD_TOP + rows as f32 * STRIP_THUMB + (rows - 1) as f32 * STRIP_GAP
}

pub fn comment_strip_height(count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    STRIP_PAD_TOP + crate::badges::BADGE_HEIGHT
}

/// Engines at or above this version understand `pending://` attachment refs
/// and QueueCommand `transfers` (send-is-a-local-write attachments). Gated on
/// BOTH the local engine (an IPC daemon may be older than this UI) and, for
/// remotely-hosted chats, the host device's stamped registry version.
const QUEUED_ATTACHMENTS_MIN: (u64, u64, u64) = (0, 2, 12);

/// What the send button is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendButtonMode {
    /// No live run: plain send.
    Send,
    /// Live run with text typed: queue for the next turn.
    Queue,
    /// Live run, nothing typed: red stop square.
    Stop,
}

/// What the composer holds that a send could carry. A staged image or diff
/// comment counts: both synthesize their own prompt body, so either alone is
/// a legal send — and during a live run has to read as Queue, not Stop.
pub fn composer_has_content(text: &str, attachments: usize, comments: usize) -> bool {
    !text.trim().is_empty() || attachments > 0 || comments > 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModifiedSubmitTarget {
    SubmitContent,
    ActivateLatestQueued,
}

fn modified_submit_target(has_content: bool) -> ModifiedSubmitTarget {
    if has_content {
        ModifiedSubmitTarget::SubmitContent
    } else {
        ModifiedSubmitTarget::ActivateLatestQueued
    }
}

pub const APPSHOT_TILE_MIN_WIDTH: f32 = 96.0;
pub const APPSHOT_IMAGE_INSET: f32 = 12.0;
pub const APPSHOT_PREVIEW_HEIGHT: f32 = 148.0;
pub const APPSHOT_IMAGE_MAX_WIDTH: f32 = 320.0;
pub const APPSHOT_IMAGE_MAX_HEIGHT: f32 = 132.0;
pub const APPSHOT_TILE_HEIGHT: f32 = 192.0;

struct AppshotActionTooltip(SharedString);

impl Render for AppshotActionTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx);
        div()
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.surface_raised)
            .shadow_md()
            .text_size(px(11.0))
            .text_color(theme.text)
            .child(self.0.clone())
    }
}

/// Give ordinary captures a shared height while their width follows the
/// source window. Extreme panoramas and narrow composers cap width without
/// cropping. Explicit dimensions also bound the native image while decoding.
pub fn appshot_contained_size(dimensions: Option<(u32, u32)>, max_width: f32) -> (f32, f32) {
    let max_width = if max_width.is_finite() {
        max_width.clamp(1.0, APPSHOT_IMAGE_MAX_WIDTH)
    } else {
        APPSHOT_IMAGE_MAX_WIDTH
    };
    let (width, height) = dimensions
        .filter(|(width, height)| *width > 0 && *height > 0)
        .unwrap_or((16, 10));
    let scale = (max_width / width as f32).min(APPSHOT_IMAGE_MAX_HEIGHT / height as f32);
    (width as f32 * scale, height as f32 * scale)
}

pub fn appshot_strip_height(count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        STRIP_PAD_TOP + APPSHOT_TILE_HEIGHT
    }
}

pub fn send_button_mode(run_live: bool, has_text: bool) -> SendButtonMode {
    match (run_live, has_text) {
        (false, _) => SendButtonMode::Send,
        (true, true) => SendButtonMode::Queue,
        (true, false) => SendButtonMode::Stop,
    }
}

/// Queue rows are represented by the queue panel until the host promotes them
/// into the transcript. They must never publish (or refresh) a local echo.
fn should_publish_optimistic_echo(queue: bool) -> bool {
    !queue
}

fn begin_interrupt(pending: &mut HashSet<String>, chat_id: &str) -> bool {
    pending.insert(chat_id.to_string())
}

fn retain_live_interrupts(pending: &mut HashSet<String>, mut is_live: impl FnMut(&str) -> bool) {
    pending.retain(|chat_id| is_live(chat_id));
}

fn interrupt_params(chat_id: &str) -> serde_json::Value {
    serde_json::json!({
        "chatId": chat_id,
        "command": { "kind": "interrupt" },
    })
}

fn escape_dismisses_completion(key: &str, completion_open: bool) -> bool {
    key == "escape" && completion_open
}

fn wizard_escape_goes_back(key: &str, input_focused: bool, input_empty: bool) -> bool {
    key == "escape" && (!input_focused || input_empty)
}

// ---------------------------------------------------------------------------
// Multiline text input (adapted from gpui examples/input.rs)
// ---------------------------------------------------------------------------

actions!(
    composer,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToLineStart,
        DeleteToLineEnd,
        Copy,
        Cut,
        Paste,
        Newline,
        MessageNewlineOrAccept,
        ModifiedSubmit,
        Submit,
        Undo,
        Redo,
        MentionTab,
    ]
);

/// How long a run of single-character edits keeps merging into one undo step.
/// A pause longer than this starts a fresh step, so undo rewinds in the
/// bursts the user actually typed rather than one character at a time.
const UNDO_COALESCE: Duration = Duration::from_millis(700);

/// Cap on retained undo steps — a long-lived composer must not grow forever.
const UNDO_LIMIT: usize = 200;

/// The literal `@` a chip displays before its file name. Projected as TEXT so
/// it shapes, wraps, and hit-tests with the label — the earlier SVG icons
/// painted into a reserved whitespace slot never sat right at text size
/// (user report). Chips read as inline code: `@name` in the mono font over
/// the code wash.
const MENTION_PREFIX: char = '@';
const MENTION_TOOLTIP_DELAY: Duration = Duration::from_millis(420);
const MENTION_TOOLTIP_HEIGHT: f32 = 24.0;
const MENTION_SIDE_PAD: &str = "\u{00A0}";
/// A private URI scheme keeps file mentions distinguishable from ordinary
/// Markdown links pasted into the composer.
use crate::workspace_links::{FILE_MENTION_SCHEME, LEGACY_FILE_MENTION_SCHEME, strip_file_scheme};

/// A restorable point in the input's history: text plus where the caret and
/// selection sat when the edit landed.
#[derive(Clone)]
struct EditSnapshot {
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
}

/// Direction of the last edit — a run only merges with edits of its own kind.
#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    Insert,
    Delete,
}

/// Events the composer wrapper listens for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerInputEvent {
    Submitted,
    ModifiedSubmitted,
    Edited,
    CursorMoved,
    ViewportChanged,
    MentionNavigate(isize),
    MentionAccept,
    /// The secondary accept (Cmd/Ctrl+Enter) on an open completion with a
    /// selection. The slash popup ATTACHES the row as a skill chip; other
    /// completions treat it exactly like [`Self::MentionAccept`].
    MentionAttach,
    MentionDismiss,
    /// Images pasted from the clipboard (screenshots / copied image data) —
    /// the wrapper stages them as attachments (use-attachments.ts onPaste).
    PastedImages(Vec<gpui::Image>),
    /// File paths pasted from the clipboard (a file manager "Copy").
    PastedPaths(Vec<PathBuf>),
}

// ---------------------------------------------------------------------------
// Composer wrapper
// ---------------------------------------------------------------------------

/// Events the shell listens for.
#[derive(Debug, Clone)]
pub enum ComposerEvent {
    SignIn { device: String, harness: mast_proto::HarnessId, account: Option<String>, snapshot: mast_proto::ManagedAccountsSnapshot },
    /// Arm the shared-element transition before the draft route is replaced
    /// by the newly-created session. Emitting this before `select_chat` keeps
    /// the first destination frame on the same timeline as the source frame.
    NewThreadTransitionStarted,
    /// A prompt was sent optimistically — give the transcript its exact row
    /// identity so it can anchor the prompt at the top with the reply's
    /// reserved space below it.
    Sent { chat_id: String, message_id: String },
    /// A new worktree's host-side setup attempt completed after its chat id
    /// was minted. The shell attaches an already-open terminal to that exact
    /// chat, even when the user has selected another chat in the meantime.
    WorktreeSetup {
        chat_id: String,
        setup_action: Option<mast_proto::ProjectActionRun>,
        setup_error: Option<String>,
        target_device_id: Option<String>,
    },
    /// A locally-authored queue row was accepted. It is not a transcript send
    /// yet: the transcript remembers the stable id and promotes it to an
    /// own-turn anchor only when the host materializes the matching bubble.
    Queued { chat_id: String, message_id: String },
}

/// What the composer's render reads from `AppState` (the rest is its own
/// fields and its child entities): the selected chat and project rows, queue,
/// review comments for the current draft key, delivery posture, device
/// presence and capabilities, context usage and whether the run is live.
#[derive(Debug, Clone, PartialEq)]
struct ComposerSlice {
    selected_chat: Option<String>,
    chat: Option<crate::pickers::ChatControlsSlice>,
    space: Option<mast_proto::Space>,
    effective_device: Option<String>,
    local_device_id: Option<String>,
    devices: Vec<(mast_proto::Device, bool)>,
    queue: Vec<mast_doc::QueuedMessage>,
    comments: Vec<crate::comments::ReviewComment>,
    code_refs: Vec<String>,
    code_ref_pending: bool,
    comment_flush_pending: bool,
    delivery_degraded: bool,
    connectivity: mast_proto::Connectivity,
    context_usage: Option<mast_proto::ContextUsage>,
    live: bool,
}

impl ComposerSlice {
    fn read(state: &AppState, key: &str, now: chrono::DateTime<chrono::Utc>) -> Self {
        let selected = state.selected_chat.as_deref();
        Self {
            selected_chat: state.selected_chat.clone(),
            chat: state
                .selected_chat_row()
                .map(crate::pickers::ChatControlsSlice::from),
            space: state.selected_space_row().cloned(),
            effective_device: state.effective_device_id(),
            local_device_id: state.local_device_id.clone(),
            devices: state
                .devices
                .iter()
                .map(|d| (d.clone(), state.device_online(&d.id, now)))
                .collect(),
            queue: state.queue.clone(),
            comments: state.review_comments(key).to_vec(),
            code_refs: state.code_refs(key).iter().map(|reference| reference.id.clone()).collect(),
            code_ref_pending: state.code_ref_load_pending(key),
            comment_flush_pending: state.review_comment_flush_pending(key),
            delivery_degraded: selected.is_some_and(|id| state.chat_delivery_degraded(id)),
            connectivity: state.connectivity.clone(),
            context_usage: state.context_usage,
            live: selected.is_some_and(|id| {
                matches!(
                    state.indicator_for(id, now),
                    Indicator::Working | Indicator::AwaitingInput
                )
            }),
        }
    }
}

#[derive(Clone)]
struct FailedSend {
    key: String,
    text: String,
    message_id: String,
    queued: bool,
    attachment_ids: Vec<String>,
    appshot_ids: Vec<String>,
    skills: Vec<String>,
    comments: Vec<crate::comments::ReviewComment>,
    code_refs: Vec<crate::code_ref::StagedCodeRef>,
}

pub struct Composer {
    sign_in_check: Option<(Option<String>, mast_proto::HarnessId, Option<String>, Instant, bool)>,
    pub(crate) state: Entity<AppState>,
    seen: crate::observe_gate::ObserveGate<ComposerSlice>,
    pub(crate) input: Entity<ComposerInput>,
    /// Draft displaced while a queued message occupies the composer.
    pub(crate) queue_edit_draft: Option<(String, Vec<StagedAttachment>, Vec<CapturedAppshot>)>,
    /// Composer actions row plus the new-session floating target tab
    /// ([`Pickers::render_new_thread_target_selectors`]).
    pickers: Entity<Pickers>,
    /// Draft text per chat key ("" = new-chat canvas), surviving navigation.
    drafts: HashMap<String, String>,
    /// Throttle for "the user is acting here" pings to the engine (orchestrator
    /// wakes hold events while the user is typing).
    activity: crate::orchestrator_wakes::ActivityReporter,
    /// Staged-but-unsent attachments per chat key (use-attachments.ts `stash`):
    /// navigating away and back restores them; memory-only, like the original.
    pub(crate) attachments: HashMap<String, Vec<StagedAttachment>>,
    /// Rich window captures keyed exactly like drafts and ordinary staged
    /// attachments. Each owns one screenshot that joins the existing upload
    /// path only at send time.
    pub(crate) appshots: HashMap<String, Vec<CapturedAppshot>>,
    /// Skills attached from the slash popup (Cmd/Ctrl+Enter or the row's
    /// ＋), keyed exactly like drafts. Per-draft: they serialize into the
    /// outgoing message's preamble ([`with_skill_preamble`]) and clear on
    /// send — the model keeps a loaded skill in context for the rest of the
    /// chat, so nothing persists per chat.
    skills: HashMap<String, Vec<String>>,
    code_ref_focus: HashMap<String, code_ref_chips::ReferenceChipFocus>,
    code_ref_focus_card: Option<(String, Entity<crate::components::code_ref::ReferenceCard>)>,
    code_ref_focus_pending: Option<String>,
    appshot_entrances: HashMap<String, Instant>,
    /// The staged attachment being viewed full-size (click a thumbnail).
    preview: Option<attachments::PreviewImage>,
    /// Focused while the lightbox is open so Escape reaches it; the input
    /// gets focus back on close.
    preview_focus: FocusHandle,
    /// Focus grab deferred to the next render (open sites don't all have a
    /// `Window` — the `MAST_ATTACH_PREVIEW` boot knob opens in `new`).
    preview_focus_pending: bool,
    /// In-flight file-picker prompt (paperclip).
    picker_task: Option<Task<()>>,
    mention_task: Option<Task<()>>,
    mention: FileMentionState,
    slash_task: Option<Task<()>>,
    slash: SlashState,
    /// Advertised commands per (harness, cwd) — one `ListCommands` per key
    /// per [`SLASH_CACHE_TTL`] (the engine caches discovery on its side too,
    /// also per-cwd for harnesses that support it).
    slash_cache: HashMap<SlashCacheKey, SlashCacheEntry>,
    /// Slash-popup row scroll — the stack overflows into a wheel/keyboard-
    /// scrollable list once it outgrows the card.
    slash_scroll: gpui::ScrollHandle,
    /// File-mention popup row scroll (same treatment).
    mention_scroll: gpui::ScrollHandle,
    /// Shared scrollbar hover/drag state for both popups' floating rails —
    /// they never show at once (mutually exclusive by token shape).
    popup_bar: crate::popover::MenuScrollbarState,
    pub(crate) current_key: String,
    sending: bool,
    /// Armed immediately before a blank-canvas send selects its minted chat.
    /// The state observer consumes it to distinguish that handoff from normal
    /// session navigation, which must continue to snap.
    launching_new_chat: bool,
    pub(crate) failure: Option<SharedString>,
    /// The chat key `failure` belongs to (`None` = global, e.g. "Engine not
    /// connected"). Chat-scoped failures survive navigation and render only
    /// under their own chat — a blanket clear-on-switch erased the one
    /// visible trace of a failed send (2026-08-19).
    failure_key: Option<String>,
    failed_send: Option<FailedSend>,
    retry_send_id: Option<String>,
    /// Work Outline instruction refusals (context reset confirmation).
    instruction_reset: instruction_reset::InstructionResetState,
    wizard: Option<Wizard>,
    wizard_focus: FocusHandle,
    /// Requests already answered locally (suppresses the panel until the doc
    /// frame marks them resolved).
    answered_requests: HashSet<String>,
    advance_task: Option<Task<()>>,
    send_task: Option<Task<()>>,
    #[cfg(test)]
    fail_send_before_dispatch: bool,
    /// The queued message being edited in the composer (see
    /// [`Composer::begin_queue_edit`]).
    pub(crate) editing_queued: Option<String>,
    /// Host-issued generation protecting `editing_queued` from automatic
    /// delivery. The text buffer is kept until Finish receives an ACK.
    pub(crate) queue_edit_lease_id: Option<String>,
    pub(crate) queue_edit_base_text_hash: Option<String>,
    pub(crate) queue_edit_chat_id: Option<String>,
    pub(crate) queue_edit_host_device_id: Option<String>,
    pub(crate) queue_edit_instance_id: String,
    pub(crate) queue_edit_pending_id: Option<String>,
    pub(crate) queue_edit_finishing: bool,
    pub(crate) queue_edit_task: Option<Task<()>>,
    pub(crate) queue_edit_renew_task: Option<Task<()>>,
    /// Focus once on mount, navigation, or after opening/closing a queue edit.
    pub(crate) focus_pending: bool,
    /// Live drag over the queue panel: which row, and where it would land.
    pub(crate) queue_drag: Option<crate::queue::QueueDragState>,
    pub(crate) queue_scroll: gpui::ScrollHandle,
    pub(crate) queue_full_preview: Option<Task<()>>,
    pub(crate) queue_previews: HashMap<(String, String), crate::queue::QueuePreview>,
    /// Rows awaiting a host-authoritative removal acknowledgement. They stay
    /// visible but inert until the host wins the race against queue delivery.
    pub(crate) queue_removing: HashSet<String>,
    /// Whether the modifier overlay should currently reveal the queue hint.
    /// The shell owns modifier tracking and clears this on window deactivation.
    queue_shortcut_revealed: bool,
    /// Interrupt/answer commands get their own slot: assigning `send_task`
    /// DROPPED an in-flight send future mid-upload — no banner, no cleanup,
    /// `sending` stuck true forever (2026-08-19 incident, "press Stop while
    /// a send grinds" shape).
    action_task: Option<Task<()>>,
    /// Chats whose durable Interrupt command has been accepted or is still
    /// being queued. Kept independently so stopping one chat cannot replace
    /// another chat's request when the user navigates quickly.
    interrupting: HashSet<String>,
    interrupt_tasks: HashMap<String, Task<()>>,
    // -- compact/expanded flip state (hysteresis; see `composer_flip`) --
    /// Current layout mode (persisted across frames — never derived fresh).
    expanded_mode: bool,
    /// `layout_epoch` of the measurement that caused the last flip: the flip is
    /// re-evaluated only after the input has been laid out in the new mode, so
    /// at most one flip can happen per layout pass.
    flip_epoch: u64,
    /// Compact-mode input capacity, learned while compact (layout-stable).
    compact_capacity: f32,
    /// Input width first measured after expanding — container-width deltas
    /// while expanded shift `compact_capacity` by the same amount.
    expanded_anchor: f32,
    /// Last input width seen in the current mode (resize detection).
    last_seen_width: f32,
    /// Stable outer composer width supplied by the shell. Unlike Taffy's
    /// provisional input measurements, this changes only when the actual
    /// conversation column changes and can safely drive a follow-up render.
    last_available_width: Option<f32>,
    /// Set while an interactive resize is in flight; collapse is deferred
    /// until widths have settled for [`RESIZE_SETTLE_MS`].
    width_changed_at: Option<Instant>,
    settle_task: Option<Task<()>>,
    /// In-flight compact↔expanded morph (one per committed flip; manual
    /// drive — see [`FlipMorph`]).
    flip_morph: Option<FlipMorph>,
    /// Pill height actually rendered last frame — a committed flip morphs
    /// from here, so mid-flight reversals hand off without a jump.
    last_rendered_height: f32,
    model_handoff_position: f32,
    model_handoff_from: f32,
    model_handoff_morph: Option<FlipMorph>,
    model_bounds: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    dock_frame: Option<crate::composer_dock::DockFrame>,
    /// The shared clock owns this frame's height, including its final step.
    dock_height_changed: bool,
    dock_clearance_correction: f32,
    surface_bounds: crate::new_thread_background_mask::SurfaceBounds,
    last_target_height: f32,
    height_morph: Option<FlipMorph>,
    /// Monotonic clock anchor for the morph timeline.
    morph_clock: Instant,
    /// Set on every session/route change: flips committed before this instant
    /// SNAP instead of morphing (see [`ROUTE_SNAP_MS`]).
    route_snap_until: Option<Instant>,
    _observe: Subscription,
    _pickers_observe: Subscription,
    _picker_focus: Subscription,
    _input_events: Subscription,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    #[cfg(feature = "snapshot")]
    pub(crate) fn fixture_attachment(&mut self, bytes: Vec<u8>) {
        let attachment = attachments::stage_png_bytes("preview.png".into(), bytes);
        self.attachments.entry(self.current_key.clone()).or_default().push(attachment);
    }
    fn refresh_sign_in_status(&mut self, cx: &mut Context<Self>) -> Option<(String, mast_proto::HarnessId, Option<String>, mast_proto::ManagedAccountsSnapshot)> {
        let state = self.state.read(cx);
        if state.selected_chat.is_some() { self.sign_in_check = None; return None; }
        let harness = self.pickers.read(cx).resolved(cx).harness?;
        if !crate::managed_accounts::supports_accounts(harness) { return None; }
        let device = state.effective_device_id();
        let missing_snapshot = state.managed_accounts_for(device.as_deref()).is_none();
        let has_space = state.selected_space_row().is_some();
        let snapshot = state.managed_accounts_for(device.as_deref()).cloned().unwrap_or_default();
        let account = state.selected_space_row().and_then(|space| crate::managed_accounts::project_account(&snapshot, &space.id, harness)).map(|account| account.id.clone());
        let key = (device.clone(), harness, account.clone());
        let refresh = self.sign_in_check.as_ref().is_none_or(|(d, h, a, at, _)| (d, *h, a) != (&key.0, key.1, &key.2) || at.elapsed() >= Duration::from_secs(30));
        if refresh {
            let rpc = state.accounts_rpc(device.as_deref());
            if missing_snapshot {
                self.state.update(cx, |state, cx| state.load_managed_accounts(device.clone(), cx));
            }
            let started = Instant::now();
            self.sign_in_check = Some((device.clone(), harness, account.clone(), started, false));
            let request_key = key.clone();
            cx.spawn(async move |this, cx| {
                let result = rpc.call(mast_rpc::methods::LOCAL_AGENT_SIGN_IN_STATUS, serde_json::json!({"harness": harness, "accountId": request_key.2.clone()})).await;
                this.update(cx, |this, cx| {
                    if let Some((d, h, a, at, signed_out)) = &mut this.sign_in_check
                        && (&*d, *h, &*a, *at) == (&request_key.0, request_key.1, &request_key.2, started) {
                            *signed_out = result.ok().and_then(|value| value.as_str().map(str::to_owned)).as_deref() == Some("signed_out");
                            cx.notify();
                        }
                }).ok();
            }).detach();
            return None;
        }
        if missing_snapshot && has_space { return None; }
        self.sign_in_check.as_ref().is_some_and(|entry| entry.4).then(|| (device.unwrap_or_default(), harness, account, snapshot))
    }
    pub(crate) fn set_dock_frame(
        &mut self,
        frame: crate::composer_dock::DockFrame,
        cx: &mut Context<Self>,
    ) {
        let changed = self.dock_frame != Some(frame);
        self.dock_height_changed |= self
            .dock_frame
            .is_none_or(|previous| previous.amount != frame.amount);
        self.dock_frame = Some(frame);
        if frame.active {
            self.flip_morph = None;
            self.height_morph = None;
        }
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn dock_clearance_correction(&self) -> f32 {
        self.dock_clearance_correction
    }

    pub(crate) fn surface_bounds(&self) -> crate::new_thread_background_mask::SurfaceBounds {
        self.surface_bounds.clone()
    }

    /// The picker entity, for the shell's canvas target selectors.
    pub fn pickers(&self) -> &Entity<Pickers> {
        &self.pickers
    }

    /// Feed the stable conversation-column width into responsive composer
    /// controls.
    pub fn set_available_width(&mut self, width: f32, cx: &mut Context<Self>) {
        let composer_width = width.clamp(0.0, COMPOSER_MAX_WIDTH);
        if composer_width_changed(self.last_available_width, composer_width) {
            self.last_available_width = Some(composer_width);
            // The shell renders before this child, so this queues one more
            // pass after the input has been laid out at its final width. That
            // pass can consume the completed measurement without emitting an
            // event from inside Taffy's multi-pass measurement callback.
            cx.notify();
        }
    }

    pub(crate) fn set_queue_shortcut_revealed(&mut self, revealed: bool, cx: &mut Context<Self>) {
        if self.queue_shortcut_revealed != revealed {
            self.queue_shortcut_revealed = revealed;
            cx.notify();
        }
    }

    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.on_release(|this, cx| this.release_queue_previews(cx))
            .detach();
        let input = cx.new(|cx| {
            let mut input =
                ComposerInput::with_context("Do anything…", MESSAGE_COMPOSER_CONTEXT, cx);
            input.enable_mentions();
            input
        });
        let pickers = cx.new(|cx| Pickers::new(state.clone(), cx));
        // The footer toolbar (checkout kind + ref picker) is rendered INLINE
        // by the composer from picker state — a pickers-side notify (refs
        // loaded, popover toggled, pick made) must repaint the composer too.
        let pickers_observe = cx.observe(&pickers, |_, _, cx| cx.notify());
        let picker_focus = cx.subscribe(
            &pickers,
            |this: &mut Self, _, _: &crate::pickers::ReturnComposerFocus, cx| {
                this.focus_pending = true;
                cx.notify();
            },
        );
        let observe = cx.observe(&state, |this: &mut Self, _, cx| this.on_state_changed(cx));
        let input_events = cx.subscribe(&input, |this: &mut Self, _, event, cx| match event {
            ComposerInputEvent::Submitted => this.on_submit(cx),
            ComposerInputEvent::ModifiedSubmitted => this.on_modified_submit(cx),
            ComposerInputEvent::Edited => this.on_input_edited(cx),
            ComposerInputEvent::CursorMoved => {
                let input = this.input.read(cx);
                if this.mention.token.is_some()
                    || this.slash.token.is_some()
                    || this.mention.dismissed.is_some()
                    || this.slash.dismissed.is_some()
                    || mention_token(input.text(), input.cursor_offset()).is_some()
                    || slash_token(input.text(), input.cursor_offset()).is_some()
                {
                    this.on_input_edited(cx);
                } else {
                    this.report_user_activity(cx);
                }
            }
            ComposerInputEvent::ViewportChanged => cx.notify(),
            // The slash popup and the mention popup share the input's
            // completion key routing; they are mutually exclusive by token
            // shape (a token opening with `/` vs `@` at a token boundary —
            // `slash_token` rejects any token containing `@`, and
            // `on_input_edited` never opens the file popup while a slash
            // token is live), so routing on `slash.token` is unambiguous.
            ComposerInputEvent::MentionNavigate(delta) => {
                if this.slash.token.is_some() {
                    this.move_slash(*delta, cx)
                } else {
                    this.move_mention(*delta, cx)
                }
            }
            ComposerInputEvent::MentionAccept => {
                if this.slash.token.is_some() {
                    this.accept_slash(cx)
                } else {
                    this.accept_mention(cx)
                }
            }
            // Cmd/Ctrl+Enter: the slash popup's secondary action attaches the
            // row as a skill chip; the file popup has no second action.
            ComposerInputEvent::MentionAttach => {
                if this.slash.token.is_some() {
                    this.attach_slash(cx)
                } else {
                    this.accept_mention(cx)
                }
            }
            ComposerInputEvent::MentionDismiss => {
                if this.slash.token.is_some() {
                    this.dismiss_slash(cx)
                } else {
                    this.dismiss_mention(cx)
                }
            }
            ComposerInputEvent::PastedImages(images) => {
                let staged = images
                    .iter()
                    .map(|image| attachments::stage_clipboard_image(image.clone()))
                    .collect();
                this.add_staged(staged, cx);
            }
            ComposerInputEvent::PastedPaths(paths) => this.add_paths(paths.clone(), cx),
        });
        let current_key = state.read(cx).selected_chat.clone().unwrap_or_default();
        let mut composer = Self {
            sign_in_check: None,
            state,
            input,
            queue_edit_draft: None,
            pickers,
            drafts: HashMap::new(),
            activity: Default::default(),
            attachments: HashMap::new(),
            appshots: HashMap::new(),
            skills: HashMap::new(),
            code_ref_focus: HashMap::new(),
            code_ref_focus_card: None,
            code_ref_focus_pending: None,
            appshot_entrances: HashMap::new(),
            preview: None,
            preview_focus: cx.focus_handle(),
            preview_focus_pending: false,
            picker_task: None,
            mention_task: None,
            mention: FileMentionState::default(),
            slash_task: None,
            slash: SlashState::default(),
            slash_cache: HashMap::new(),
            slash_scroll: gpui::ScrollHandle::new(),
            mention_scroll: gpui::ScrollHandle::new(),
            popup_bar: crate::popover::MenuScrollbarState::default(),
            current_key,
            sending: false,
            launching_new_chat: false,
            failure: None,
            seen: Default::default(),
            wizard: None,
            wizard_focus: cx.focus_handle(),
            answered_requests: HashSet::new(),
            failure_key: None,
            failed_send: None,
            retry_send_id: None,
            instruction_reset: Default::default(),
            action_task: None,
            advance_task: None,
            send_task: None,
            #[cfg(test)]
            fail_send_before_dispatch: false,
            interrupting: HashSet::new(),
            interrupt_tasks: HashMap::new(),
            editing_queued: None,
            queue_edit_lease_id: None,
            queue_edit_base_text_hash: None,
            queue_edit_chat_id: None,
            queue_edit_host_device_id: None,
            queue_edit_instance_id: uuid::Uuid::new_v4().to_string(),
            queue_edit_pending_id: None,
            queue_edit_finishing: false,
            queue_edit_task: None,
            queue_edit_renew_task: None,
            focus_pending: true,
            queue_drag: None,
            queue_scroll: gpui::ScrollHandle::new(),
            queue_full_preview: None,
            queue_previews: HashMap::new(),
            queue_removing: HashSet::new(),
            queue_shortcut_revealed: false,
            expanded_mode: false,
            flip_epoch: 0,
            compact_capacity: 0.0,
            expanded_anchor: 0.0,
            last_seen_width: 0.0,
            last_available_width: None,
            width_changed_at: None,
            settle_task: None,
            flip_morph: None,
            last_rendered_height: 0.0,
            model_handoff_position: 1.0,
            model_handoff_from: 1.0,
            model_handoff_morph: None,
            model_bounds: Default::default(),
            dock_frame: None,
            dock_height_changed: false,
            dock_clearance_correction: 0.0,
            surface_bounds: Default::default(),
            last_target_height: 0.0,
            height_morph: None,
            morph_clock: Instant::now(),
            route_snap_until: None,
            _observe: observe,
            _pickers_observe: pickers_observe,
            _picker_focus: picker_focus,
            _input_events: input_events,
        };
        // Dev knob: pre-stage attachments (drop/paste can't be synthesized on
        // a rig) — `MAST_ATTACH=/path/a.png[,/path/b.png]`, and
        // `MAST_ATTACH_PREVIEW=1` boots with the first one's lightbox open.
        if let Ok(spec) = mast_proto::env::var("MAST_ATTACH") {
            let staged: Vec<StagedAttachment> = spec
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .filter_map(|path| {
                    match attachments::stage_file(std::path::Path::new(path.trim())) {
                        Ok(att) => Some(att),
                        Err(err) => {
                            tracing::warn!(%path, error = %err, "MAST_ATTACH stage failed");
                            None
                        }
                    }
                })
                .collect();
            if mast_proto::env::var("MAST_ATTACH_PREVIEW").is_ok_and(|v| v == "1")
                && let Some(first) = staged.first()
            {
                composer.preview = Some(attachments::PreviewImage::new(
                    first.name.clone(),
                    first.image.clone(),
                ));
                composer.preview_focus_pending = true;
            }
            if !staged.is_empty() {
                composer
                    .attachments
                    .entry(composer.current_key.clone())
                    .or_default()
                    .extend(staged);
            }
        }
        composer
    }

    /// Capture-knob passthrough (`MAST_OPEN_DIALOG=model`): open the
    /// combined harness/model menu.
    pub fn debug_open_model_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pickers
            .update(cx, |pickers, cx| pickers.open_model_menu(window, cx));
    }

    /// The prompt input's current draft (tests).
    #[cfg(test)]
    pub(crate) fn draft_text(&self, cx: &App) -> String {
        self.input.read(cx).text().to_owned()
    }

    pub fn is_sending(&self) -> bool {
        self.sending
    }

    pub(crate) fn can_edit_queue_in_composer(&self) -> bool {
        !self.sending && self.wizard.is_none()
    }

    // ---- attachment staging (use-attachments.ts) ----

    /// Staged attachments for the chat the composer is showing.
    pub(crate) fn staged(&self) -> &[StagedAttachment] {
        self.attachments
            .get(&self.current_key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub(crate) fn queue_preview_limit(&self) -> usize {
        if self.last_available_width.unwrap_or(COMPOSER_MAX_WIDTH) < 520.0 {
            1
        } else {
            2
        }
    }

    pub(crate) fn show_queue_image(
        &mut self,
        preview: attachments::PreviewImage,
        cx: &mut Context<Self>,
    ) {
        self.preview = Some(preview);
        self.preview_focus_pending = true;
        cx.notify();
    }

    pub(crate) fn staged_appshots(&self) -> &[CapturedAppshot] {
        self.appshots
            .get(&self.current_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Skills attached to the draft the composer is showing.
    fn staged_skills(&self) -> &[String] {
        self.skills
            .get(&self.current_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn remove_skill(&mut self, name: &str, cx: &mut Context<Self>) {
        let key = self.current_key.clone();
        if let Some(list) = self.skills.get_mut(&key)
            && detach_skill(list, name)
        {
            if list.is_empty() {
                self.skills.remove(&key);
            }
            self.focus_pending = true;
            cx.notify();
        }
    }

    /// The attached-skills row: one removable chip per skill, above the input
    /// (same strip geometry as the review-comments chip, so the pill's
    /// arithmetic sizing stays exact — see [`comment_strip_height`]).
    /// Reconcile composer-owned state with `AppState` (draft swap on
    /// navigation, question panel, queue-edit and interrupt bookkeeping), and
    /// repaint only when that changed something visible: the slice of state
    /// the composer renders differs, or the reconcile itself moved composer
    /// state. Streaming frames and other chats' updates do neither.
    fn on_state_changed(&mut self, cx: &mut Context<Self>) {
        let before = self.reconcile_signature();
        self.reconcile_with_state(cx);
        let slice = ComposerSlice::read(self.state.read(cx), &self.current_key, chrono::Utc::now());
        let slice_changed = self.seen.changed(slice);
        if slice_changed || before != self.reconcile_signature() {
            cx.notify();
        }
    }

    /// The composer-owned fields `reconcile_with_state` can change.
    fn reconcile_signature(&self) -> impl PartialEq + use<> {
        (
            self.current_key.clone(),
            self.wizard.as_ref().map(|w| w.request_id.clone()),
            self.editing_queued.clone(),
            self.failure.clone(),
            self.interrupting.len(),
            self.queue_removing.len(),
        )
    }

    fn reconcile_with_state(&mut self, cx: &mut Context<Self>) {
        {
            let state = self.state.read(cx);
            let now = chrono::Utc::now();
            retain_live_interrupts(&mut self.interrupting, |chat_id| {
                matches!(
                    state.indicator_for(chat_id, now),
                    Indicator::Working | Indicator::AwaitingInput
                )
            });
            self.queue_removing
                .retain(|id| state.queue.iter().any(|item| item.id == *id));
        }
        self.interrupt_tasks
            .retain(|chat_id, _| self.interrupting.contains(chat_id));

        let editing_id = self.editing_queued.clone();
        let (key, pending, edited_row_exists) = {
            let s = self.state.read(cx);
            (
                s.selected_chat.clone().unwrap_or_default(),
                pending_input_request(&s.transcript),
                editing_id
                    .as_ref()
                    .is_none_or(|id| s.queue.iter().any(|item| item.id == *id)),
            )
        };

        // A queue edit belongs to exactly one visible row. Navigation or a
        // remote drain/removal cancels it instead of leaving a focused but
        // unmounted editor entity behind.
        if key != self.current_key && self.editing_queued.is_some() {
            self.clear_queue_edit(cx);
        } else if !edited_row_exists && self.editing_queued.is_some() && !self.queue_edit_finishing
        {
            self.failure =
                Some("The queued message was removed; your edit remains in the composer".into());
            // Recover both drafts when another device removes the reserved row.
            if let Some((draft, mut attachments, mut appshots)) = self.queue_edit_draft.take() {
                appshots.extend(self.appshots.remove(&self.current_key).unwrap_or_default());
                self.appshots.insert(self.current_key.clone(), appshots);
                let edited = self.input.read(cx).text().to_string();
                let text = [draft, edited]
                    .into_iter()
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                self.input.update(cx, |input, cx| input.set_text(text, cx));
                attachments.extend(
                    self.attachments
                        .remove(&self.current_key)
                        .unwrap_or_default(),
                );
                self.attachments
                    .insert(self.current_key.clone(), attachments);
            }
            self.clear_queue_edit(cx);
        }

        // Draft swap on chat navigation — the input entity itself survives.
        if key != self.current_key {
            let new_thread_launch =
                self.launching_new_chat && self.current_key.is_empty() && !key.is_empty();
            let returning_to_new_thread = !self.current_key.is_empty() && key.is_empty();
            self.launching_new_chat = false;
            let old_text = self.input.read(cx).text().to_string();
            if old_text.is_empty() {
                self.drafts.remove(&self.current_key);
            } else {
                self.drafts.insert(self.current_key.clone(), old_text);
            }
            let draft = self.drafts.get(&key).cloned().unwrap_or_default();
            self.current_key = key;
            // `failure` deliberately survives navigation: chat-scoped
            // failures render only under their own chat (see `failure_key`),
            // so switching away and back must not erase the one visible
            // trace of a failed send.
            self.wizard = None;
            // Attachments stay stashed under their chat key (the map swap IS
            // the navigation); only the transient chrome resets.
            self.preview = None;
            self.reset_mention(None, cx);
            // Route changes snap (round 5/6): a mode difference between the
            // old and new session's composer must not glide across
            // navigation. Killing the in-flight morph here isn't enough —
            // the nav-driven flip only commits AFTER the swapped draft has
            // been re-measured, one or two renders later, so the whole
            // window snaps (see ROUTE_SNAP_MS).
            self.height_morph = None;
            self.last_target_height = 0.0;
            if (new_thread_launch || returning_to_new_thread)
                && !motion::reduced_motion(cx)
                && self.last_rendered_height > 0.0
            {
                // Both directions share one timeline. The blank canvas is
                // always expanded; an established session begins compact.
                self.expanded_mode = returning_to_new_thread;
                let now_ms =
                    self.morph_clock.elapsed().as_secs_f32() * 1000.0 / motion::speed_scale();
                self.flip_morph = Some(FlipMorph::new_thread_transition(
                    self.last_rendered_height,
                    now_ms,
                ));
                self.route_snap_until = None;
            } else {
                self.flip_morph = None;
                self.last_rendered_height = 0.0;
                self.route_snap_until = Some(Instant::now() + Duration::from_millis(ROUTE_SNAP_MS));
            }
            self.input.update(cx, |input, cx| input.set_text(draft, cx));
            // Landing on a chat is activity there; the restored draft decides
            // the flag.
            self.report_user_activity(cx);
        }

        // A pending agent question must not take over an active queue edit.
        if self.editing_queued.is_some() {
            return;
        }
        // Question panel lifecycle (wizard state cached per request id).
        match pending {
            Some((request_id, questions)) if !self.answered_requests.contains(&request_id) => {
                let same = self
                    .wizard
                    .as_ref()
                    .is_some_and(|w| w.request_id == request_id);
                if !same {
                    self.reset_mention(None, cx);
                    self.wizard = Some(Wizard::new(request_id, questions));
                    self.advance_task = None;
                    // The shared input becomes the panel's free-text override.
                    self.input.update(cx, |input, cx| {
                        input.set_placeholder("Type your own answer, or pick an option above", cx)
                    });
                }
            }
            _ => {
                if let Some(wizard) = self.wizard.as_ref() {
                    // LATCH (original composer.tsx `inputLatch`): a transient
                    // fold/sync blip — or a steer appended behind the
                    // streaming entry — must not unmount the panel and lose
                    // the user's picks. Release only on explicit resolution
                    // (here or on another device) or when a NON-EMPTY
                    // transcript shows the question superseded (a newer
                    // assistant entry took over). Never on run death: the
                    // question stays answerable until answered — the engine
                    // delivers a dead run's answer as a resumed turn.
                    let transcript = self.state.read(cx).transcript.clone();
                    let released = input_request_resolved(&transcript, &wizard.request_id)
                        || (!transcript.is_empty()
                            && !self.answered_requests.contains(&wizard.request_id));
                    if released {
                        self.wizard = None;
                        self.advance_task = None;
                        self.input
                            .update(cx, |input, cx| input.set_placeholder("Do anything…", cx));
                    }
                }
            }
        }
        let input_context = message_input_context(self.wizard.is_some());
        self.input
            .update(cx, |input, cx| input.set_key_context(input_context, cx));
    }

    pub(crate) fn run_live(&self, cx: &App) -> bool {
        let s = self.state.read(cx);
        let Some(chat_id) = s.selected_chat.as_deref() else {
            return false;
        };
        matches!(
            s.indicator_for(chat_id, chrono::Utc::now()),
            Indicator::Working | Indicator::AwaitingInput
        )
    }

    /// New chats need a runnable agent, but may target the device's home
    /// directory without a project. Existing chats carry their own run config.
    fn send_blocked(&self, cx: &App) -> bool {
        if self.queue_edit_finishing {
            return true;
        }
        let state = self.state.read(cx);
        if state.review_comment_flush_pending(&self.current_key) || state.code_ref_load_pending(&self.current_key) {
            return true;
        }
        if state.selected_chat.is_some() {
            return false;
        }
        // New-chat canvas: needs a runnable agent. The
        // no-agents check only fires once the catalog is loaded — offline
        // and still-loading states must not block (the harness resolves from
        // the remembered default and the engine reports real failures).
        self.pickers.read(cx).no_agents_available()
    }

    fn button_mode(&self, cx: &App) -> SendButtonMode {
        if self.editing_queued.is_some() {
            return SendButtonMode::Send;
        }
        let has_text = composer_has_content(
            self.input.read(cx).text(),
            self.staged().len() + self.staged_appshots().len() + self.staged_skills().len(),
            self.staged_comments(cx).len() + self.state.read(cx).code_refs(&self.current_key).iter().filter(|reference| reference.reference.kind != mast_proto::CodeRefKind::File).count()
                + usize::from(self.state.read(cx).code_ref_load_pending(&self.current_key)),
        );
        send_button_mode(self.run_live(cx), has_text)
    }

    fn on_submit(&mut self, cx: &mut Context<Self>) {
        if self.commit_queue_edit(cx) {
            return;
        }
        if self.wizard.is_some() {
            // Enter inside the panel's free-text input submits the page.
            let typed = self.input.read(cx).text().trim().to_string();
            if let Some(w) = self.wizard.as_mut() {
                w.set_typed(typed);
            }
            self.wizard_advance(cx);
            return;
        }
        let text = self.input.read(cx).text().trim().to_string();
        let no_content = !composer_has_content(
            &text,
            self.staged().len() + self.staged_appshots().len() + self.staged_skills().len(),
            self.staged_comments(cx).len() + self.state.read(cx).code_refs(&self.current_key).iter().filter(|reference| reference.reference.kind != mast_proto::CodeRefKind::File).count(),
        );
        match self.button_mode(cx) {
            // Enter never stops a run: Stop mode implies an empty composer,
            // so a stray extra Enter right after sending landed an interrupt
            // on the just-dispatched prompt and the agent ate it silently
            // (issue #406). Stop stays on the button — and on Esc when
            // escape_stops_active_agent is enabled.
            SendButtonMode::Stop => {}
            _ if no_content => {}
            _ if self.send_blocked(cx) => {}
            SendButtonMode::Send => self.send(text, false, cx),
            // Busy: keep the message queued until the current turn ends.
            SendButtonMode::Queue => self.send(text, true, cx),
        }
    }

    /// Cmd/Ctrl+Enter remains an ordinary submit while the composer carries
    /// content. With a truly empty composer it instead activates the most
    /// recently queued row, and never turns an empty chord into Stop.
    fn on_modified_submit(&mut self, cx: &mut Context<Self>) {
        if self.commit_queue_edit(cx) {
            return;
        }
        let has_content = composer_has_content(
            self.input.read(cx).text(),
            self.staged().len() + self.staged_appshots().len() + self.staged_skills().len(),
            self.staged_comments(cx).len() + self.state.read(cx).code_refs(&self.current_key).iter().filter(|reference| reference.reference.kind != mast_proto::CodeRefKind::File).count(),
        );
        match modified_submit_target(has_content) {
            ModifiedSubmitTarget::SubmitContent => self.on_submit(cx),
            ModifiedSubmitTarget::ActivateLatestQueued => self.activate_latest_queued(cx),
        }
    }

    fn restore_failed_text(&mut self, key: &str, text: &str, error: &str, cx: &mut Context<Self>) {
        self.failure = Some(error.to_owned().into());
        self.failure_key = Some(key.to_owned());
        let merge = |existing: &str| {
            if existing.is_empty() { text.to_owned() }
            else if text.is_empty() { existing.to_owned() }
            else { format!("{text}\n\n{existing}") }
        };
        if self.current_key == key {
            let restored = merge(self.input.read(cx).text());
            self.input.update(cx, |input, cx| input.set_text(restored, cx));
        } else {
            let restored = merge(self.drafts.get(key).map(String::as_str).unwrap_or(""));
            self.drafts.insert(key.to_owned(), restored);
        }
    }

    pub(crate) fn interrupt_selected(&mut self, cx: &mut Context<Self>) {
        let Some(chat_id) = self.state.read(cx).selected_chat.clone() else {
            return;
        };
        self.interrupt_chat(chat_id, cx);
    }

    pub(crate) fn interrupt_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        if !begin_interrupt(&mut self.interrupting, &chat_id) {
            return;
        }
        let params = interrupt_params(&chat_id);
        let task_chat_id = chat_id.clone();
        let failure_chat = chat_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = engine.client().call(methods::QUEUE_COMMAND, params).await;
            if let Err(err) = result {
                this.update(cx, |composer, cx| {
                    composer.interrupting.remove(&task_chat_id);
                    composer.failure = Some(format!("Stop failed: {err}").into());
                    composer.failure_key = Some(failure_chat);
                    cx.notify();
                })
                .ok();
            }
        });
        self.interrupt_tasks.insert(chat_id, task);
    }

    pub(crate) fn is_interrupting(&self, chat_id: &str) -> bool {
        self.interrupting.contains(chat_id)
    }

    // ---- wizard glue ----

    fn render_send_button(
        &mut self,
        mode: SendButtonMode,
        identity: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::of(cx);
        let stop = mode == SendButtonMode::Stop;
        let blocked = !stop && self.send_blocked(cx);
        crate::components::conversation_send_key(theme, identity, stop, mode == SendButtonMode::Queue)
            .when(blocked, |el| el.opacity(0.35))
            .when(!blocked, |el| {
                el.cursor_pointer().hover(|style| style.opacity(0.85))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if stop { this.interrupt_selected(cx); } else { this.on_submit(cx); }
                    }))
            })
            .into_any_element()
    }
}

/// The completion popups' floating rails run through
/// [`crate::popover::rail`]: the shared `popup_bar` state plus whichever
/// popup's rows are mounted — see [`Composer::active_popup_scroll`].
impl crate::popover::ScrollRailHost for Composer {
    fn rail_bar(&mut self) -> &mut crate::popover::MenuScrollbarState {
        &mut self.popup_bar
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.active_popup_scroll()
    }
}

/// Focus lands on the prompt input (window-level focus fallbacks — e.g. after
/// the focused terminal panel is hidden — route here).
impl Focusable for Composer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_pending {
            self.focus_pending = false;
            let focus = self.input.focus_handle(cx);
            window.focus(&focus, cx);
        }
        let theme = Theme::of(cx).clone();
        let wizard_active = self.wizard.is_some();
        let pickers = self.pickers.read(cx);
        let identity = theme.agent_identity_color(pickers.effective_harness(cx), pickers.identity_model_id(cx));
        let identity_text = crate::components::conversation_input_text_color(&theme, identity);
        if self.mention.token.is_some()
            && (wizard_active || !self.input.focus_handle(cx).is_focused(window))
        {
            self.reset_mention(None, cx);
        }
        if self.slash.token.is_some()
            && (wizard_active || !self.input.focus_handle(cx).is_focused(window))
        {
            self.reset_slash(None, cx);
        }
        let mode = self.button_mode(cx);
        let text_size = crate::components::conversation_composer_text_size(&theme, INPUT_TEXT_SIZE);
        // Shape the current draft before sizing the pill. Waiting for the child
        // layout leaves the parent using the previous edit's height.
        self.input.update(cx, |input, cx| {
            if input.text_size != text_size {
                input.text_size = text_size;
                input.needs_measure = true;
            }
            if input.identity_text != identity_text {
                input.identity_text = identity_text;
                input.needs_measure = true;
            }
            if input.needs_measure && input.last_width > 0.0 {
                let mut style = window.text_style();
                style.font_family = theme.font_sans.clone();
                style.font_size = crate::typography::ui_rems(text_size).into();
                style.color = if let Some(color) = identity_text { color } else if input.content.is_empty() {
                    theme.text_faint
                } else {
                    theme.text
                };
                input.layout_text(px(input.last_width), &style, window, cx);
            }
        });
        let (text_width, has_newline, content_height, last_width, epoch) = {
            let input = self.input.read(cx);
            (
                input.measured_text_width(),
                input.has_newline(),
                input.measured_content_height(),
                input.last_width,
                input.layout_epoch,
            )
        };
        let content_height = crate::components::conversation_composer_content_height(&theme, content_height, INPUT_LINE_HEIGHT);
        let now = Instant::now();
        // Only measurements taken *after* the last flip may drive the next one
        // (at most one flip per layout pass — a flip invalidates the widths).
        let measured_since_flip = epoch > self.flip_epoch && last_width > 0.0;
        if measured_since_flip {
            // A same-mode width change is an interactive window/pane resize:
            // defer collapse until sizes settle for RESIZE_SETTLE_MS. Expansion
            // remains live so compact controls never squeeze the input away.
            if self.last_seen_width > 0.0 && (last_width - self.last_seen_width).abs() > 0.5 {
                self.width_changed_at = Some(now);
            }
            self.last_seen_width = last_width;
            if self.expanded_mode {
                if self.expanded_anchor <= 0.0 {
                    self.expanded_anchor = last_width;
                }
            } else {
                // The compact pill's content box is the layout-stable capacity
                // both thresholds measure against.
                self.compact_capacity = last_width - 8.0;
            }
        }
        let resizing = self
            .width_changed_at
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(RESIZE_SETTLE_MS));
        if resizing && self.settle_task.is_none() {
            // Re-evaluate once the settle window has passed.
            self.settle_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(RESIZE_SETTLE_MS + 20))
                    .await;
                this.update(cx, |composer, cx| {
                    composer.settle_task = None;
                    cx.notify();
                })
                .ok();
            }));
        }
        // Layout-stable compact capacity: measured directly while compact;
        // while expanded, the learned value shifted by any container resize
        // (the expanded input width tracks the container 1:1).
        let capacity = if !self.expanded_mode {
            if last_width > 0.0 {
                last_width - 8.0
            } else {
                f32::MAX // before first measure default to compact
            }
        } else if self.compact_capacity > 0.0 {
            if self.expanded_anchor > 0.0 && last_width > 0.0 {
                self.compact_capacity + (last_width - self.expanded_anchor)
            } else {
                self.compact_capacity
            }
        } else {
            f32::MAX
        };
        let next = composer_flip(
            self.expanded_mode,
            text_width,
            capacity,
            has_newline,
            resizing,
        );
        let committed_flip = next != self.expanded_mode && measured_since_flip;
        if committed_flip {
            self.expanded_mode = next;
            self.flip_epoch = epoch;
            self.expanded_anchor = 0.0;
            // The mode change moves the input width; don't read that jump as
            // an interactive resize.
            self.last_seen_width = 0.0;
        }
        // New chats render expanded regardless of `expanded_mode` (see below),
        // so a mode flip there changes nothing visible — never morph it.
        let new_chat = self.state.read(cx).selected_chat.is_none();
        let sign_in_notice = self.refresh_sign_in_status(cx);
        // Morph clock in ms; dividing by the measurement knob stretches the
        // timeline exactly like shell.rs eval_tween's scaled duration.
        let now_ms = self.morph_clock.elapsed().as_secs_f32() * 1000.0 / motion::speed_scale();
        let route_snap = self
            .route_snap_until
            .is_some_and(|until| Instant::now() < until);
        let dock_height_changed = std::mem::take(&mut self.dock_height_changed);
        self.flip_morph =
            if dock_height_changed || self.dock_frame.is_some_and(|frame| frame.active) {
                None
            } else {
                flip_morph_step(
                    self.flip_morph,
                    committed_flip && !new_chat,
                    self.last_rendered_height,
                    now_ms,
                    motion::reduced_motion(cx),
                    route_snap,
                )
            };
        let expanded = crate::components::conversation_composer_expanded(
            &theme,
            self.expanded_mode,
            self.surface_bounds.get().map(|bounds| f32::from(bounds.size.width)),
        );

        // Chat-scoped failures render only under their own chat; a global
        // failure (no key) renders everywhere.
        let failure = self.failure.clone().filter(|_| {
            self.failure_key
                .as_ref()
                .is_none_or(|key| *key == self.current_key)
        });
        let retry = self.failed_send.clone().filter(|failed| {
            let allowed = failure.as_ref().is_some_and(|message| {
                let lower = message.to_lowercase();
                !mast_harness::is_authentication_failure(message)
                    && !["capability", "policy", "unsupported", "not supported", "refused", "permission denied"].iter().any(|word| lower.contains(word))
            });
            allowed && !self.sending && failed.key == self.current_key
                && self.input.read(cx).text().trim() == failed.text
                && self.staged().iter().map(|item| &item.id).eq(failed.attachment_ids.iter())
                && self.staged_appshots().iter().map(|item| &item.id).eq(failed.appshot_ids.iter())
                && self.staged_skills() == failed.skills
                && self.staged_comments(cx) == failed.comments
                && self.state.read(cx).code_refs(&self.current_key) == failed.code_refs
        });
        // Composer honesty: when the target's delivery path is degraded, say
        // UP FRONT that a send will queue (a durable local write delivered on
        // reconnect) instead of letting the button imply instant delivery.
        let queue_notice: Option<(SharedString, bool)> = {
            use mast_proto::ConnectivityState as S;
            let state = self.state.read(cx);
            let degraded = match state.selected_chat.as_deref() {
                Some(id) => state.chat_delivery_degraded(id),
                None => {
                    // New-chat canvas: judge by the picked target device.
                    let remote_target = state
                        .effective_device_id()
                        .is_some_and(|id| state.local_device_id.as_deref() != Some(id.as_str()));
                    remote_target
                        && (matches!(state.connectivity.state, S::Offline | S::Reconnecting)
                            || state
                                .effective_device_id()
                                .is_some_and(|id| !state.device_online(&id, chrono::Utc::now())))
                }
            };
            let offline = state.connectivity.state == S::Offline;
            degraded.then(|| {
                let text: SharedString = if offline {
                    "Offline — messages will send when you're back online.".into()
                } else {
                    "Messages will send once the connection recovers.".into()
                };
                (text, offline)
            })
        };
        let context_reset = self.render_context_reset_prompt(&theme, cx);
        // Centered composer column (zeron `mx-auto w-full max-w-3xl`).
        let container = div()
            .w_full()
            .max_w(px(crate::components::conversation_composer_width(&theme, COMPOSER_MAX_WIDTH, Theme::SPACE_LG)))
            .mx_auto()
            .flex()
            .flex_col()
            .gap(px(Theme::SPACE_SM))
            .px(px(Theme::SPACE_LG))
            .pb(px(Theme::SPACE_LG))
            .when_some(failure, |el, message| {
                // Amber "Warning" chip for the offline-ish case (engine not
                // connected); everything else is a send/run/worktree failure
                // and gets the persistent, copyable error card (full detail,
                // Copy button) that stays until dismissed.
                let offline = message.as_ref() == "Engine not connected";
                if offline {
                    return el.child(
                        notice_chip(&theme, true, "Warning", message, NoticeChipIcon::Plain)
                            .id("composer-failure")
                            .mx(px(4.0))
                            .mt(px(6.0))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.failure = None;
                                this.failure_key = None;
                                cx.notify();
                            })),
                    );
                }
                let composer = cx.entity();
                el.child(
                    div()
                        .id("composer-failure")
                        .debug_selector(|| "composer-failure".into())
                        .mx(px(4.0))
                        .mt(px(6.0))
                        .child(
                            crate::error_notice::ErrorNotice::from_text(
                                "composer-failure-card",
                                &theme,
                                &message,
                            )
                            .on_dismiss(move |_, cx| {
                                composer.update(cx, |this, cx| {
                                    this.failure = None;
                                    this.failure_key = None;
                                    cx.notify();
                                })
                            }),
                        ),
                )
            })
            .when_some(retry, |el, failed| el.child(
                div().id("composer-retry").debug_selector(|| "composer-retry".into())
                    .cursor_pointer().text_color(theme.accent).child("Retry")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.sending { return; }
                        this.retry_send_id = Some(failed.message_id.clone());
                        this.send(failed.text.clone(), failed.queued, cx);
                    }))
            ))
            .children(context_reset)
            .when_some(queue_notice, |el, (notice, offline)| {
                // Not a warning box (v0.2.12 feedback: the amber Notice read
                // as an error and flashed on every blip — pre-grace). One
                // quiet caption line, amber dot only for hard offline; it
                // clears itself the moment the path heals.
                let dot = if offline {
                    theme.warning
                } else {
                    theme.text_faint
                };
                el.child(crate::motion::fade_in(
                    "composer-queue-notice",
                    div()
                        .id("composer-queue-notice")
                        .mx(px(8.0))
                        .mt(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(11.0))
                        .line_height(px(14.0))
                        .text_color(theme.text_faint)
                        .child(div().size(px(5.0)).rounded_full().bg(dot))
                        .child(div().min_w_0().truncate().child(notice)),
                ))
            });

        if wizard_active {
            let wizard = self.render_wizard(window, cx);
            return container.child(motion::fade_quick("composer-wizard", div().child(wizard)));
        }

        // What is waiting to be sent, stacked directly above the box it was
        // typed in — the queue is a property of this composer, not a panel
        // somewhere else.
        let show_queue_latest_shortcut = self.queue_shortcut_revealed
            && self.editing_queued.is_none()
            && !self.pickers.read(cx).is_open()
            && !composer_has_content(
                self.input.read(cx).text(),
                self.staged().len() + self.staged_appshots().len() + self.staged_skills().len(),
                self.staged_comments(cx).len() + self.state.read(cx).code_refs(&self.current_key).iter().filter(|reference| reference.reference.kind != mast_proto::CodeRefKind::File).count(),
            );
        let container = container.when_some(
            self.render_queue_panel(show_queue_latest_shortcut, window, cx),
            |el, panel| {
                el.child(motion::fade_quick(
                    "composer-queue",
                    div()
                        .mx(px(QUEUE_SIDE_INSET))
                        // Cancel the column gap, then tuck the tray one pixel
                        // behind the composer painted after it.
                        .mb(px(-(Theme::SPACE_SM + QUEUE_COMPOSER_OVERLAP)))
                        .child(panel),
                ))
            },
        );
        // Escape backs out of a queue-row edit (the row keeps its old text).
        // Bound here rather than in the input: the input's own Escape belongs
        // to the mention/slash popups, which outrank this while they're open.
        let container = container.when(self.editing_queued.is_some(), |el| {
            el.on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape"
                    && this.mention.token.is_none()
                    && this.slash.token.is_none()
                    && this.cancel_queue_edit(cx)
                {
                    cx.stop_propagation();
                }
            }))
        });

        // Route coordination must not force an established thread into the
        // two-row layout. Short drafts keep the original skinny composer.
        let session_expanded = expanded;
        let expanded = expanded || new_chat;
        let dock_amount = self.dock_frame.map_or(0.0, |frame| frame.amount);
        let dock_height = |amount: f32| {
            let hero = composer_total_height(content_height);
            let session = if session_expanded {
                (content_height + TEXTAREA_PAD_V).clamp(TEXTAREA_MIN - 16.0, TEXTAREA_MAX)
                    + ACTIONS_ROW_HEIGHT
                    + PILL_BORDER_V
            } else {
                COMPACT_TOTAL_HEIGHT
            };
            motion::lerp(hero, session, amount)
        };

        // Committed-height morph: the layout below is already the NEW mode's;
        // only the pill's height (and the entrance fade/text glide driven by
        // `morph_t`) animates. Steady state renders exactly the target.
        // Staged attachments add the wrap strip's height to the pill in BOTH
        // modes (attachment-ui.tsx AttachmentStrip sits above the input row).
        let staged_count = self.staged().len();
        // The input width excludes the inline controls in compact mode.
        // Wrap against the pill's content width in both modes, accounting
        // for the outer container padding and the pill's 1px borders.
        let strip_width_hint =
            self.last_available_width.unwrap_or(COMPOSER_MAX_WIDTH) - 2.0 * Theme::SPACE_LG - 2.0;
        let appshot_count = self.staged_appshots().len();
        let strip_h = attachment_strip_height(staged_count, strip_width_hint);
        // The attached-skills row shares the comment chip's strip geometry;
        // both fold into one height term so every sizing site below agrees.
        let comment_strip_h = comment_strip_height(self.staged_comments(cx).len())
            + comment_strip_height(self.staged_skills().len())
            + comment_strip_height(self.state.read(cx).code_refs(&self.current_key).iter().filter(|reference| reference.reference.kind != mast_proto::CodeRefKind::File).count());
        let base_height = if self.dock_frame.is_some() {
            dock_height(dock_amount)
        } else if expanded {
            composer_total_height(content_height)
        } else {
            COMPACT_TOTAL_HEIGHT
        };
        let target_height =
            base_height + strip_h + appshot_strip_height(appshot_count) + comment_strip_h;
        let coordinated_route_morph = self
            .flip_morph
            .filter(|m| m.spec == motion::NEW_THREAD_TRANSITION && !m.done(now_ms));
        // The route state commits before its shared-element animation begins.
        // Reconstruct the departing chrome at t=0, then progressively trade
        // it for the destination chrome so neither route changes the outer
        // composer geometry in a single frame.
        let new_thread_chrome = self
            .dock_frame
            .map(|frame| frame.selectors())
            .unwrap_or_else(|| {
                coordinated_route_morph.map_or_else(
                    || if new_chat { 1.0 } else { 0.0 },
                    |morph| {
                        let progress = morph.progress(now_ms);
                        if new_chat { progress } else { 1.0 - progress }
                    },
                )
            });
        let (new_thread_chrome_opacity, session_chrome_opacity) = self.dock_frame.map_or_else(
            || route_chrome_opacities(new_thread_chrome),
            |frame| (frame.selectors(), frame.footer()),
        );
        self.height_morph =
            if dock_height_changed || self.dock_frame.is_some_and(|frame| frame.active) {
                None
            } else if coordinated_route_morph.is_some() {
                coordinated_route_morph
            } else {
                flip_morph_step(
                    self.height_morph,
                    (target_height - self.last_target_height).abs() > 0.5,
                    self.last_rendered_height,
                    now_ms,
                    motion::reduced_motion(cx),
                    route_snap,
                )
            };
        self.last_target_height = target_height;
        let pill_height = self
            .height_morph
            .map_or(target_height, |m| m.height(target_height, now_ms));
        if self.height_morph.is_some() {
            window.request_animation_frame();
        }
        let (_, morph_t, morphing) = match self.flip_morph {
            Some(m) if !m.done(now_ms) => {
                (m.height(target_height, now_ms), m.progress(now_ms), true)
            }
            _ => (target_height, 1.0, false),
        };
        if !morphing {
            self.flip_morph = None;
        } else {
            // Manual tween drive: keep frames coming (shell.rs motion_active).
            window.request_animation_frame();
        }
        self.last_rendered_height = pill_height;
        self.dock_clearance_correction = self.dock_frame.map_or(0.0, |frame| {
            dock_height(if frame.docked { 1.0 } else { 0.0 })
                + strip_h
                + appshot_strip_height(appshot_count)
                + comment_strip_h
                - pill_height
        });
        // Route morphs use the dock's reversible clock; typing flips keep
        // their existing local clock once the composer reaches its dock.
        let layout_morph_t =
            if self.dock_frame.is_some_and(|frame| frame.active) && !session_expanded {
                if expanded {
                    1.0 - dock_amount
                } else {
                    dock_amount
                }
            } else {
                morph_t
            };
        let text_pt = morph_text_pad(layout_morph_t);
        let surface_radius = crate::components::conversation_panel_radius(&theme, COMPOSER_RADIUS - 4.0 * dock_amount);
        let route_to_single_line =
            self.dock_frame.is_some_and(|frame| frame.active) && !session_expanded;
        let textarea_height = (pill_height
            - strip_h
            - appshot_strip_height(appshot_count)
            - comment_strip_h
            - PILL_BORDER_V
            - ACTIONS_ROW_HEIGHT)
            .max(if route_to_single_line {
                INPUT_LINE_HEIGHT + text_pt + 4.0
            } else {
                0.0
            });
        self.input.update(cx, |input, cx| {
            let height = if expanded {
                (textarea_height - text_pt - 4.0).max(0.0)
            } else {
                INPUT_LINE_HEIGHT
            };
            let settled_height = if expanded {
                (base_height - PILL_BORDER_V - ACTIONS_ROW_HEIGHT - TEXTAREA_PAD_V).max(
                    if route_to_single_line {
                        INPUT_LINE_HEIGHT
                    } else {
                        0.0
                    },
                )
            } else {
                INPUT_LINE_HEIGHT
            };
            let resizing = self.height_morph.is_some();
            let top_padding = if expanded { text_pt } else { 0.0 };
            if input.viewport_height != Some(height)
                || input.settled_viewport_height != Some(settled_height)
                || input.resizing != resizing
                || input.overflow_top_padding != top_padding
            {
                input.resizing = resizing;
                input.overflow_top_padding = top_padding;
                input.viewport_height = Some(height);
                input.settled_viewport_height = Some(settled_height);
                cx.notify();
            }
        });

        let send_button = self.render_send_button(mode, identity, cx);
        // Attach button — opens the native image picker (the original's hidden
        // `<input type=file accept="image/*" multiple>`); paste/drop also feed
        // the same strip. The leading utility group owns the spacing between
        // this button and the model picker.
        let attach = crate::components::conversation_attach_key(&theme)
            .on_click(cx.listener(|this, _, _, cx| this.open_file_picker(cx)));
        // Staged-thumbnail strip (attachment-ui.tsx AttachmentStrip), above
        // the input inside the pill in both modes.
        let strip = self.render_attachment_strip(&theme, cx);
        let appshot_strip = self.render_appshot_strip(&theme, window, cx);
        let comments_chip = self.render_comments_chip(&theme, cx);
        let code_refs = self.render_code_refs(&theme, window, cx);
        let skill_chips = self.render_skill_chips(&theme, cx);

        // A translucent cool silver/slate edge sits more naturally on frost
        // than the general-purpose white/black separator color.
        let pill_border = if theme.is_frost() {
            match theme.appearance {
                crate::theme::Appearance::Dark => gpui::hsla(210.0 / 360.0, 0.18, 0.78, 0.09),
                crate::theme::Appearance::Light => gpui::hsla(210.0 / 360.0, 0.18, 0.32, 0.10),
            }
        } else {
            theme.border
        };
        // Compensate for the transcript canvas beneath the frosted surface.
        // Keep the opaque fallback when frost is disabled or unsupported.
        let pill = crate::components::conversation_input_surface(div()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    // Padding and action controls are part of the text composer.
                    // Open menus keep their own keyboard/search focus.
                    if !this.pickers.read(cx).is_open() {
                        window.focus(&this.input.focus_handle(cx), cx);
                    }
                }),
            ), &theme, identity, surface_radius, pill_border);
        // The pill's bottom edge is stationary on screen (the composer sits at
        // the bottom of the shell column; growth moves the TOP edge), so the
        // controls pin to the bottom and only the text glides with the reveal
        // (round-9 follow-up: the send/attach/chips must not ride the height,
        // while the model picker fades between its two horizontal anchors).
        let cluster_dy = morph_cluster_dy(layout_morph_t);
        // Share the height/route timeline instead of starting an independent
        // animation. Reversals continue from the current handoff phase.
        if self.model_handoff_morph != self.flip_morph {
            self.model_handoff_from = self.model_handoff_position;
            self.model_handoff_morph = self.flip_morph;
        }
        let compact_target = if expanded { 0.0 } else { 1.0 };
        self.model_handoff_position =
            if self.dock_frame.is_some_and(|frame| frame.active) && !session_expanded {
                dock_amount
            } else {
                self.flip_morph.map_or(compact_target, |morph| {
                    motion::lerp(
                        self.model_handoff_from,
                        compact_target,
                        motion::EASE_IN_OUT.eval(morph.raw(now_ms)),
                    )
                })
            };
        let surface_width = self
            .surface_bounds
            .get()
            .map_or(strip_width_hint + PILL_BORDER_V, |bounds| {
                f32::from(bounds.size.width)
            });
        let send_hint = crate::components::conversation_send_hint(&theme, mode != SendButtonMode::Stop, surface_width,
            match crate::settings::current(cx).composer_send_behavior {
                crate::settings::ComposerSendBehavior::Enter => "enter",
                crate::settings::ComposerSendBehavior::ModEnter => "mod-enter",
            });
        let model_travel = (surface_width
            - PILL_BORDER_V
            - 12.0
            - 28.0
            - ACTION_UTILITY_GAP
            - self
                .model_bounds
                .get()
                .map_or(0.0, |bounds| f32::from(bounds.size.width))
            - ACTION_PRIMARY_GAP
            - 28.0
            - morph_cluster_inset(expanded, layout_morph_t))
        .max(0.0);
        let (model_side, model_opacity, model_drift) = model_handoff(self.model_handoff_position);
        let model_offset = (model_side - compact_target) * model_travel + model_drift;
        let measured_model_bounds = self.model_bounds.clone();
        let (model_picker, horizon_picker) = crate::components::conversation_model_picker(
            &theme, surface_width, model_offset, model_opacity, self.pickers.clone().into_any_element(),
                gpui::canvas(
                    move |bounds, _, _| measured_model_bounds.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0().into_any_element(),
        );
        let horizon_tabs = {
            let selected = self.pickers.read(cx).effective_harness(cx);
            let harnesses = self.pickers.read(cx).horizon_harnesses(cx);
            let tabs = harnesses.into_iter().map(|harness| {
                let (path, _) = crate::pickers::harness_brand_icon(harness);
                let selected = selected == Some(harness);
                let tab_identity = theme.agent_identity_color(Some(harness), Some("selected"));
                crate::components::conversation_harness_tab(&theme, SharedString::from(format!("horizon-harness-{harness:?}")), tab_identity, identity, path, selected)
                    .on_click(cx.listener(move |this, _, window, cx| this.pickers.update(cx, |pickers, cx| {
                        pickers.pick_harness(harness, cx);
                        pickers.open_model_menu(window, cx);
                    }))).into_any_element()
            }).collect::<Vec<_>>();
            let add = crate::components::conversation_add_harness_key(&theme)
                    .on_click(cx.listener(|this, _, window, cx| this.pickers.update(cx, |pickers, cx| pickers.open_model_menu(window, cx))))
                    .into_any_element();
            crate::components::conversation_harness_tabs(&theme, tabs, add)
        };
        let text_glide = if self.dock_frame.is_some_and(|frame| frame.active) {
            collapse_text_glide(dock_height(0.0), dock_amount)
        } else {
            match self.flip_morph {
                Some(m) if morphing => collapse_text_glide(m.from, morph_t),
                _ => 0.0,
            }
        };
        let strips = code_refs.into_iter().map(|el| el.into_any_element())
            .chain(comments_chip.into_iter().map(|el| el.into_any_element()))
            .chain(skill_chips.into_iter().map(|el| el.into_any_element()))
            .chain(appshot_strip)
            .chain(strip.into_iter().map(|el| el.into_any_element()))
            .collect();
        let body = crate::components::conversation_composer_body(&theme, crate::components::ConversationComposerBody {
            pill, input: self.render_input_with_completion().into_any_element(), strips,
            attach: attach.into_any_element(), send: send_button,
            model: model_picker.map(|el| el.into_any_element()),
            horizon_model: horizon_picker.map(|el| el.into_any_element()),
            send_hint: send_hint.map(|el| el.into_any_element()),
            expanded, pill_height, textarea_height, text_padding: text_pt,
            cluster_offset: cluster_dy,
            expanded_inset: morph_cluster_inset(true, layout_morph_t),
            compact_inset: morph_cluster_inset(false, layout_morph_t),
            text_glide, surface_width,
        });
        let new_thread_target_selectors = (new_thread_chrome_opacity > 0.0).then(|| {
            self.pickers.update(cx, |pickers, cx| {
                pickers.render_new_thread_target_selectors(cx)
            })
        });
        let new_thread_git_selectors = (new_thread_chrome_opacity > 0.0)
            .then(|| {
                self.pickers.update(cx, |pickers, cx| {
                    pickers.render_new_thread_git_selectors(cx)
                })
            })
            .flatten();
        let has_new_thread_git_selectors = self
            .state
            .read(cx)
            .selected_space_row()
            .is_some_and(|space| space.git_detected);
        // The file dropzone lives in the shell (the whole conversation column,
        // not just the pill — shell.rs `chat-dropzone`); drops land back here
        // via `add_paths`.
        // Frosted: the pill backdrop-blurs the transcript scrolling under it
        // (the popover glass treatment; radius matches the pill's rounding).
        // The shell keeps this entity under one parent on both routes. The
        // surface itself never fades, and frost follows the same morph radius.
        let pill_surface = div()
            .relative()
            .id("composer-surface")
            .debug_selector(|| "composer-surface".into())
            .child(crate::frost::frosted(surface_radius, 16.0, body))
            .children(horizon_tabs)
            .child({
                let measured = self.surface_bounds.clone();
                // All prepaint completes before any paint. The background
                // reads this cell during paint, never last frame's geometry.
                gpui::canvas(
                    move |bounds, _, _| measured.set(Some(bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
            })
            // Both completion popups span the full pill width above it —
            // the file-mention and slash tokens are mutually exclusive.
            .children(self.render_file_mention_popup(&theme, cx))
            .children(self.render_slash_popup(&theme, cx));
        // Restore the original chip-only selector treatment: destination at
        // the top-right, no surrounding surface. Cancel the column gap as the
        // row collapses so the pill never jumps at the route boundary.
        let container = if self.dock_frame.is_some() {
            // Floating selectors share the surface's origin and never change its height.
            container.relative().child(
                div()
                    .id("dock-target-selectors")
                    .absolute()
                    .top(px(-28.0))
                    .left(px(Theme::SPACE_LG + 10.0))
                    .right(px(Theme::SPACE_LG + 10.0))
                    .h(px(NEW_THREAD_SELECTOR_ROW_HEIGHT))
                    .flex()
                    .items_start()
                    .justify_end()
                    .opacity(new_thread_chrome_opacity)
                    .children(new_thread_target_selectors),
            )
        } else if new_thread_chrome > 0.0 {
            container.child(
                div()
                    .w_full()
                    .h(px(NEW_THREAD_SELECTOR_ROW_HEIGHT * new_thread_chrome))
                    .mb(px(-Theme::SPACE_SM * (1.0 - new_thread_chrome)))
                    .px(px(10.0))
                    .flex()
                    .items_start()
                    .justify_end()
                    .opacity(new_thread_chrome_opacity)
                    .children(new_thread_target_selectors),
            )
        } else {
            container
        };
        let container = container.when_some(sign_in_notice, |container, (device, harness, account, snapshot)| {
            let name = if harness == mast_proto::HarnessId::ClaudeCode { "Claude Code" } else { "Codex" };
            container.child(div().debug_selector(|| "composer-sign-in-notice".into()).flex().items_center().gap(px(8.0))
                .text_color(theme.warning).child(SharedString::from(format!("{name} isn't signed in")))
                .child(crate::popover::btn_ghost(&theme, "Sign in…", "composer-sign-in")
                    .id("composer-sign-in").debug_selector(|| "composer-sign-in".into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sign_in_check = None;
                        cx.emit(ComposerEvent::SignIn { device: device.clone(), harness, account: account.clone(), snapshot: snapshot.clone() });
                    })))
                .child(crate::popover::btn_ghost(&theme, "Start anyway", "composer-start-anyway")
                    .id("composer-start-anyway").debug_selector(|| "composer-start-anyway".into())
                    .on_click(cx.listener(|this, _, _, cx| this.on_submit(cx)))))
        }).child(pill_surface);

        // The lower slot keeps a stable footprint for Git projects while its
        // old floating checkout/ref controls dissolve into the session footer.
        // Non-Git sessions grow the slot continuously from zero.
        let session_chrome = 1.0 - new_thread_chrome;
        let bottom_slot = if has_new_thread_git_selectors || self.dock_frame.is_some() {
            1.0
        } else {
            session_chrome
        };
        let container = if bottom_slot > 0.0 {
            let footer = (session_chrome_opacity > 0.0).then(|| {
                self.pickers
                    .update(cx, |pickers, cx| pickers.render_footer(cx))
            });
            let usage = self.state.read(cx).context_usage;
            let usage = (session_chrome_opacity > 0.0 && crate::context_usage::has_window(usage)).then(||
                crate::context_usage::render(usage, &theme).into_any_element());
            container.child(crate::components::conversation_footer_slot(
                SESSION_FOOTER_HEIGHT, bottom_slot, new_thread_chrome_opacity, session_chrome_opacity,
                new_thread_git_selectors.map(|el| el.into_any_element()),
                footer.flatten().map(|el| el.into_any_element()), usage,
            ))
        } else {
            container
        };
        // Full-size preview of a staged thumbnail (AttachmentPreviewDialog).
        if let Some(preview) = self.preview.clone() {
            if std::mem::take(&mut self.preview_focus_pending) {
                window.focus(&self.preview_focus, cx);
            }
            let weak = cx.weak_entity();
            return container.child(attachments::lightbox(
                window,
                &preview,
                &self.preview_focus,
                move |window, cx| {
                    // Hand focus back to the input so typing (and the next
                    // Escape) lands where it did before the lightbox opened.
                    if let Ok(input_focus) = weak.update(cx, |this, cx| {
                        this.preview = None;
                        cx.notify();
                        this.input.read(cx).focus_handle.clone()
                    }) {
                        window.focus(&input_focus, cx);
                    }
                },
                cx,
            ));
        }
        container
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod appshot_rebase_tests;

#[cfg(feature = "appshots-fixture")]
impl Composer {
    pub fn fixture_clear_appshots(&mut self, cx: &mut Context<Self>) {
        self.appshots.clear();
        cx.notify();
    }
}
