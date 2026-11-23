//! PR/ticket linkage for a chat's branch — ticket 002 phase 2.
//!
//! Ported from `session_canvas_server.py` (the `agent-mode-tools` stopgap
//! tool)'s subprocess-shell-out approach, not a Rust API client: `gh pr view`
//! and `linear issue view`, matching `_run_gh_pr_view`/`fetch_linear_ticket`.
//! Rides the user's own `gh`/`linear` CLI auth; avoids standing up REST/
//! GraphQL types for no real benefit over a CLI that already works. `linear`
//! is optional — not installed on every machine (confirmed: absent on this
//! one) — its absence degrades to "no ticket data" rather than an error.
//!
//! Contract, matching the reference implementation's `get_cached_pr`/
//! `get_cached_ticket`: the read path (`status_for_chat`) is a **pure,
//! non-blocking cache read** — it never itself shells out. A miss just
//! registers the key for the next background sweep and returns `None` for
//! that piece; the caller (or a later poll) sees it populated once the sweep
//! catches up. The sweep (`spawn_sweep_loop`) drains a few pending keys per
//! tick — matches the reference's `PR_KEYS_PER_SWEEP`/`TICKET_IDS_PER_SWEEP
//! = 3` throttling spirit so this never hammers `gh`/`linear` unthrottled.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How long a cached PR/ticket entry is trusted before the sweep refreshes
/// it. The reference implementation's exact TTL constants weren't re-derived
/// here (not read as part of this pass) — this is a reasonable value in the
/// same spirit: fresh enough for an overview glanced at repeatedly, cheap
/// enough not to hammer `gh`/`linear`.
const CACHE_TTL: Duration = Duration::from_secs(180);
/// How long a cached diff-stat entry is trusted — much shorter than
/// `CACHE_TTL`: unlike a PR/ticket lookup (a remote call worth throttling
/// hard), a worktree's uncommitted-changes stat is local, cheap, and changes
/// as often as the session itself edits files, so parity with the ticket's
/// "modest TTL (~20-30s)" ask matters more here than reusing the GH/Linear
/// cadence.
const DIFF_STAT_TTL: Duration = Duration::from_secs(25);
/// Sweep tick interval.
/// How long a "no PR found" result stays fresh. Most worktree branches have no PR, and
/// each miss costs a `gh pr view` in the checkout plus one per submodule, so re-asking
/// every `CACHE_TTL` kept the sweep busy forever. A PR Mast opens is linked durably (by
/// URL), so this only delays noticing a PR opened elsewhere.
const NEGATIVE_PR_TTL: Duration = Duration::from_secs(600);
/// A PR state with no successful check for this long is no longer served (a merged PR is
/// terminal and exempt).
const STALE_PR_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);
/// Backoff between lookups of one key after a transient `gh` failure.
const PR_RETRY_BASE: Duration = Duration::from_secs(30);
const PR_RETRY_CEILING: Duration = Duration::from_secs(15 * 60);

/// What one throttled sweep lookup found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PrLookup {
    Found(PrStatus),
    /// `gh` said the PR does not exist.
    NotFound,
    /// Transient failure (network, auth, no `gh`): keep what we know.
    Error,
}

/// A PR cache batch result: what changed, for subscribers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCacheUpdate {
    pub revision: u64,
    /// PR URLs whose cached state changed (old and new).
    pub urls: Vec<String>,
    /// Branch lookups whose result changed (checkout path and branch), so the
    /// chats on those branches can be refreshed.
    pub branches: Vec<PrBranchKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrBranchKey {
    pub cwd: String,
    pub branch: String,
}

const SWEEP_INTERVAL: Duration = Duration::from_secs(20);
/// Opted-in completion observations are discovered at most once per three minutes.
const COMPLETION_SCAN_INTERVAL: Duration = Duration::from_secs(180);
const COMPLETION_SCAN_LIMIT: usize = 12;
/// Pending keys drained per sweep tick, per kind (PR, ticket, diff-stat) —
/// matches the reference's `PR_KEYS_PER_SWEEP`/`TICKET_IDS_PER_SWEEP = 3`.
const KEYS_PER_SWEEP: usize = 3;

/// `gh pr view --json` field set, matching `PR_VIEW_JSON_FIELDS` in
/// `session_canvas_server.py:1399`.
const PR_VIEW_JSON_FIELDS: &str = "number,url,state,isDraft,reviewDecision,statusCheckRollup,reviews,title,headRefName,headRefOid,mergeable,reviewRequests,author,mergedAt,additions,deletions";

/// Checks-rollup summary, matching `summarize_checks` (`session_canvas_server.py:1377`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChecksStatus {
    Passing,
    Pending,
    Failing,
}

/// What, if anything, the PR's author can actually do next — the
/// `ACTION_META`/`ACTION_SEVERITY` vocabulary from `session_canvas.html`
/// (~line 2064), most-actionable first. A draft PR never classifies (matches
/// `classify_pr_action`'s early return on `is_draft`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrAction {
    CiFailing,
    MergeConflict,
    ChangesRequested,
    NeedsReviewer,
    ReadyToMerge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    #[serde(default)]
    pub additions: Option<u64>,
    #[serde(default)]
    pub deletions: Option<u64>,
    #[serde(default)]
    pub merged_at: Option<String>,
    pub number: u64,
    pub url: Option<String>,
    pub state: String,
    pub is_draft: bool,
    pub review_decision: Option<String>,
    pub reviewers: Vec<String>,
    pub checks: Option<ChecksStatus>,
    pub title: Option<String>,
    pub branch: Option<String>,
    /// Last successfully fetched PR head; absent when unavailable.
    #[serde(default)]
    pub head_sha: Option<String>,
    pub mergeable: String,
    pub has_reviewer_requested: bool,
}

impl PrStatus {
    /// Mirrors `classify_pr_action` (`session_canvas_server.py:1405`) —
    /// built here so phase 5's "My PRs" pane doesn't need to re-derive it.
    /// Only the single highest-severity reason is returned (the reference
    /// implementation returns every matching reason; here `Vec` order is
    /// severity-first, ready to render one badge or all of them).
    pub fn action_reasons(&self) -> Vec<PrAction> {
        if self.is_draft {
            return Vec::new();
        }
        let mut reasons = Vec::new();
        if self.checks == Some(ChecksStatus::Failing) {
            reasons.push(PrAction::CiFailing);
        }
        if self.mergeable == "conflicting" {
            reasons.push(PrAction::MergeConflict);
        }
        if self.review_decision.as_deref() == Some("CHANGES_REQUESTED") {
            reasons.push(PrAction::ChangesRequested);
        }
        if !self.has_reviewer_requested && self.reviewers.is_empty() {
            reasons.push(PrAction::NeedsReviewer);
        }
        if reasons.is_empty()
            && self.review_decision.as_deref() == Some("APPROVED")
            && self.checks == Some(ChecksStatus::Passing)
            && self.mergeable == "mergeable"
        {
            reasons.push(PrAction::ReadyToMerge);
        }
        reasons
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketStatus {
    pub identifier: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub status: Option<String>,
}

/// Uncommitted-changes summary for a worktree chat's cwd, from
/// `git diff --shortstat HEAD` — matches `git_diff_stat`
/// (`session_canvas_server.py:312-338`): uncommitted-only (never staged vs.
/// HEAD, never untracked files), scoped to the working tree, not the whole
/// repo history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffStat {
    pub files_changed: u32,
    pub lines_added: u32,
    pub lines_removed: u32,
}

/// One durable PR link and whatever detail the non-blocking cache currently
/// knows about it.  Keeping the URL/source even while `detail` is still
/// pending lets the overview render every attached PR immediately instead of
/// collapsing cache misses back into a single anonymous slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrLinkStatus {
    pub url: String,
    pub source: Option<mast_proto::ChatLinkSource>,
    pub detail: Option<PrStatus>,
}

/// What's known right now for one chat's branch — either side may still be
/// `None` because nothing links (no PR/ticket exists) or because the sweep
/// hasn't gotten to it yet; the caller can't distinguish those without
/// polling again, matching the reference implementation's own cache-read
/// contract.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLinkStatus {
    pub pr: Option<PrStatus>,
    /// All durable PRs attached to this chat.  `pr`/`pr_source` remain as a
    /// backwards-compatible primary mirror for older clients.
    #[serde(default)]
    pub pr_links: Vec<PrLinkStatus>,
    pub ticket: Option<TicketStatus>,
    /// `cwd` sits under a `.worktrees/`/`.agent-worktrees/` directory —
    /// `is_worktree` (`session_canvas_server.py:300-304`). A pure string
    /// check computed inline on every read, unlike everything else on this
    /// struct: no I/O, so no cache entry (and no "pending") makes sense for it.
    pub is_worktree: bool,
    /// Uncommitted-diff summary, populated ONLY when `is_worktree` — matches
    /// the reference's own gate (`_build_claude_session_tile`,
    /// server.py:2192-2257): a non-worktree chat's cwd is the primary
    /// checkout, where "uncommitted changes" isn't the same
    /// this-session's-own-sandbox signal a worktree's is. `None` while the
    /// sweep hasn't gotten to it yet (or the cwd isn't a worktree at all),
    /// same cache-read contract as `pr`/`ticket`.
    pub diff_stat: Option<DiffStat>,
    /// Provenance of `pr`, when it came from the chat row's durable
    /// `linked_pr_url` (ticket 0xx) rather than the branch-based `gh pr view`
    /// guess. `None` (serialized as JSON `null`, never omitted — see the
    /// wire-contract test below) means `pr` (when present at all) came from
    /// inference — the pre-existing behavior, unchanged for a chat with no
    /// durable link.
    pub pr_source: Option<mast_proto::ChatLinkSource>,
    /// Sibling to [`Self::pr_source`] for `ticket`.
    pub ticket_source: Option<mast_proto::ChatLinkSource>,
}

/// `is_worktree` (`session_canvas_server.py:300-304`): the cwd sits under a
/// `.worktrees/` or `.agent-worktrees/` directory anywhere along its path —
/// covers both Mast's own worktree layout and agent-mode.sh's legacy one.
pub fn is_worktree_cwd(cwd: &str) -> bool {
    cwd.contains("/.worktrees/") || cwd.contains("/.agent-worktrees/")
}

/// How long the bulk "my open PRs" list is trusted before a re-search.
/// Matches the reference's separate, cheaper cadence for the list call vs.
/// per-PR detail (`my_prs_refresh_loop`'s comment: "cheap enough to redo
/// every minute outright").
const MY_PRS_LIST_TTL: Duration = Duration::from_secs(60);

/// After a failed `gh search prs` (non-zero exit, unparseable output, or `gh`
/// missing), wait this long before trying that search again. Without it a
/// failing search was retried on every 20s sweep tick forever, which is what
/// trips GitHub's secondary rate limit.
const SEARCH_FAILURE_BACKOFF: Duration = Duration::from_secs(90);

/// One row from `gh search prs --author=@me` — list-only fields, no detail
/// (checks/review/mergeable) yet. Matches `fetch_my_open_prs`'s dict shape
/// (`session_canvas_server.py:1554`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyOpenPr {
    pub number: u64,
    pub title: Option<String>,
    pub url: Option<String>,
    pub repo: Option<String>,
    pub is_draft: bool,
    pub updated_at: Option<String>,
    /// PR author's login. Only populated by the review-requested search
    /// (where "who asked me" matters); omitted from the wire entirely for
    /// the authored list, which keeps that payload byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// A `MyOpenPr` plus whatever detail (checks/reviews/mergeable →
/// `action_reasons()`) the throttled detail sweep has filled in so far —
/// `None` until that catches up, same "may still be pending" contract as
/// `ChatLinkStatus`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MyPrItem {
    #[serde(flatten)]
    pub summary: MyOpenPr,
    pub detail: Option<PrStatus>,
}

/// Cache/pending key for a PR lookup. `Url` resolves a chat's durable
/// `linked_pr_url` (ticket 0xx) directly — no local checkout needed, unlike
/// `Branch`, which shells out from a cwd the way this cache always has.
/// `status_for` prefers `Url` when the chat has one; a link-less chat keeps
/// exactly the pre-existing `Branch` behavior.
#[derive(Clone, Hash, Eq, PartialEq)]
enum PrKey {
    Branch { cwd: PathBuf, branch: String },
    CheckoutBranch { cwd: PathBuf, branch: String },
    Url(String),
}

struct Entry<T> {
    data: Option<T>,
    fetched_at: Instant,
}

impl<T> Entry<T> {
    /// TTL is per-kind (`CACHE_TTL` for PR/ticket, `DIFF_STAT_TTL` for
    /// diff-stat) rather than a single constant baked in here.
    fn fresh(&self, ttl: Duration) -> bool {
        self.fetched_at.elapsed() < ttl
    }
}

impl Entry<PrStatus> {
    /// Freshness for a branch/URL PR lookup: a miss is kept longer than a hit.
    fn fresh_pr(&self) -> bool {
        self.fresh(if self.data.is_none() { NEGATIVE_PR_TTL } else { CACHE_TTL })
    }

    /// An expired hit that may still be shown while a refresh runs.
    fn servable_stale(&self) -> bool {
        self.data.as_ref().is_some_and(|status| status.state.eq_ignore_ascii_case("merged")
            || self.fetched_at.elapsed() < STALE_PR_LIMIT)
    }

    fn is_merged(&self) -> bool {
        self.data.as_ref().is_some_and(|status| status.state.eq_ignore_ascii_case("merged"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TicketKey {
    Cli(String),
    Linear { identifier: String, account_id: Option<String>, credential_revision: u64 },
}

struct Inner {
    merged_wakes: Mutex<Option<crate::pr_merged_wakes::PrMergedWakes>>,
    completion_scan: Mutex<(Option<Instant>, usize)>,
    pr_cache: Mutex<HashMap<PrKey, Entry<PrStatus>>>,
    ticket_cache: Mutex<HashMap<TicketKey, Entry<TicketStatus>>>,
    pending_pr: Mutex<HashSet<PrKey>>,
    pr_revision: tokio::sync::watch::Sender<PrCacheUpdate>,
    /// Failure count and earliest next lookup per PR key after a transient error.
    pr_retry: Mutex<HashMap<PrKey, (u32, Instant)>>,
    pending_ticket: Mutex<HashSet<TicketKey>>,
    linear_ticket_revisions: Mutex<HashMap<Option<String>, u64>>,
    /// Keyed by cwd — a local, no-throttling-concern lookup (unlike
    /// `pr_cache`'s `(cwd, branch)`, since diff-stat doesn't need a branch).
    diff_stat_cache: Mutex<HashMap<PathBuf, Entry<DiffStat>>>,
    pending_diff_stat: Mutex<HashSet<PathBuf>>,
    /// `None` until the first successful `fetch_my_open_prs` sweep.
    my_prs_list: Mutex<Option<(Vec<MyOpenPr>, Instant)>>,
    my_pr_detail: Mutex<HashMap<(String, u64), Entry<PrStatus>>>,
    /// When the authored search last failed (`None` = last attempt succeeded
    /// or none yet); gates retries by `SEARCH_FAILURE_BACKOFF`.
    my_prs_failed_at: Mutex<Option<Instant>>,
    /// Becomes true after the first authored-list search finishes, whether it
    /// populated the list or recorded a failure. RPC can briefly await this to
    /// avoid racing the sweep's immediate first tick.
    my_prs_ready: tokio::sync::watch::Sender<bool>,
    /// `gh search prs --review-requested=@me` results. Own slot and TTL,
    /// swept on a different tick from the authored search.
    review_prs_list: Mutex<Option<(Vec<MyOpenPr>, Instant)>>,
    review_prs_failed_at: Mutex<Option<Instant>>,
    /// Linear integration: the API client and the registry (to know which
    /// Spaces have Linear on). Tickets asked for by chats in such Spaces are
    /// fetched through the API when a key is set; the `linear` CLI stays
    /// the fallback.
    linear: Mutex<
        Option<(
            crate::linear::LinearService,
            crate::workspace_host::WorkspaceHost,
        )>,
    >,
}

/// Cheap to clone (an `Arc` around the shared cache) — hand a clone to the
/// RPC layer and to `spawn_sweep_loop`, same shape as `ExternalSessionImporter`.
#[derive(Clone)]
pub struct PrTicketCache {
    inner: Arc<Inner>,
}

impl PrTicketCache {
    /// Read a cached PR state without scheduling a refresh or a git probe.
    pub(crate) fn cached_pr_state(&self, url: &str) -> Option<String> {
        lock(&self.inner.pr_cache).get(&PrKey::Url(url.to_owned()))
            // A merged PR cannot reopen; closed and open states still need a
            // fresh observation before they influence an archive decision.
            .filter(|entry| entry.fresh_pr() || entry.data.as_ref().is_some_and(|status| status.state.eq_ignore_ascii_case("merged")))
            .and_then(|entry| entry.data.as_ref().map(|status| status.state.clone()))
    }
    /// Read a fresh durable-link result without scheduling any work. The outer
    /// option is absent when the result is unknown; the inner option is absent
    /// when a lookup completed and found no PR.
    pub fn peek_pr_url(&self, url: &str) -> Option<Option<PrStatus>> {
        let cache = lock(&self.inner.pr_cache);
        cache.get(&PrKey::Url(url.to_owned()))
            .filter(|entry| entry.fresh_pr())
            .map(|entry| entry.data.clone())
    }

    /// Queue a refresh unless the entry is a merged PR (terminal) or the key is
    /// backing off after a transient failure. Never hold `pr_cache` when calling.
    fn queue_pr_refresh(&self, key: PrKey) {
        // Only a PR URL is terminal once merged; a branch can be reused for a follow-up PR.
        if matches!(key, PrKey::Url(_)) && lock(&self.inner.pr_cache).get(&key).is_some_and(Entry::is_merged) { return; }
        if lock(&self.inner.pr_retry).get(&key).is_some_and(|(_, next)| Instant::now() < *next) { return; }
        lock(&self.inner.pending_pr).insert(key);
    }

    /// Read a cached item PR, scheduling the existing throttled sweep on a
    /// miss. Invalid URLs and a full pending queue are left unknown.
    pub fn request_pr_url(&self, url: &str) -> Option<Option<PrStatus>> {
        let canonical = crate::work_outline_pr_links::pull_request_from_url(url)?.url;
        if let Some(result) = self.peek_pr_url(url) { return Some(result); }
        if let Some(result) = self.peek_pr_url(&canonical) { return Some(result); }
        let stale = {
            let cache = lock(&self.inner.pr_cache);
            [url, canonical.as_str()].into_iter()
                .find_map(|key| cache.get(&PrKey::Url(key.to_owned())).filter(|entry| entry.servable_stale()).and_then(|entry| entry.data.clone()))
        };
        if lock(&self.inner.pending_pr).len() < 512 { self.queue_pr_refresh(PrKey::Url(canonical)); }
        stale.map(Some)
    }

    /// Changes after a batch of PR cache results is stored by the existing sweep.
    pub fn watch_pr_revision(&self) -> tokio::sync::watch::Receiver<PrCacheUpdate> {
        self.inner.pr_revision.subscribe()
    }

    /// Read an already cached branch lookup without scheduling git or network
    /// work. `checkout` selects the same namespace used for checkout targets.
    pub fn peek_pr_branch(&self, cwd: &str, branch: &str, checkout: bool) -> Option<Option<PrStatus>> {
        let key = if checkout {
            PrKey::CheckoutBranch { cwd: PathBuf::from(cwd), branch: branch.to_owned() }
        } else {
            PrKey::Branch { cwd: PathBuf::from(cwd), branch: branch.to_owned() }
        };
        let cache = lock(&self.inner.pr_cache);
        cache.get(&key).filter(|entry| entry.fresh_pr()).map(|entry| entry.data.clone())
    }

    pub fn new() -> Self {
        let (my_prs_ready, _) = tokio::sync::watch::channel(false);
        let (pr_revision, _) = tokio::sync::watch::channel(PrCacheUpdate::default());
        Self {
            inner: Arc::new(Inner {
                merged_wakes: Mutex::new(None),
                completion_scan: Mutex::new((None, 0)),
                pr_cache: Mutex::new(HashMap::new()),
                ticket_cache: Mutex::new(HashMap::new()),
                pending_pr: Mutex::new(HashSet::new()),
                pr_revision,
                pr_retry: Mutex::new(HashMap::new()),
                pending_ticket: Mutex::new(HashSet::new()),
                linear_ticket_revisions: Mutex::new(HashMap::new()),
                diff_stat_cache: Mutex::new(HashMap::new()),
                pending_diff_stat: Mutex::new(HashSet::new()),
                my_prs_list: Mutex::new(None),
                my_pr_detail: Mutex::new(HashMap::new()),
                my_prs_failed_at: Mutex::new(None),
                my_prs_ready,
                review_prs_list: Mutex::new(None),
                review_prs_failed_at: Mutex::new(None),
                linear: Mutex::new(None),
            }),
        }
    }

    fn select_completion_observations(&self, observations: &[crate::pr_merged_wakes::CompletionObservation]) -> Vec<crate::pr_merged_wakes::CompletionObservation> {
        if observations.is_empty() { return Vec::new(); }
        let mut scan = lock(&self.inner.completion_scan);
        let start = scan.1 % observations.len();
        let count = observations.len().min(COMPLETION_SCAN_LIMIT);
        scan.1 = (start + count) % observations.len();
        (0..count).map(|index| observations[(start + index) % observations.len()].clone()).collect()
    }

    fn queue_completion_observations(&self, observations: &[crate::pr_merged_wakes::CompletionObservation]) {
        let cache = lock(&self.inner.pr_cache);
        let mut pending = lock(&self.inner.pending_pr);
        for observation in observations {
            let key = match observation {
                crate::pr_merged_wakes::CompletionObservation::Url(url) => PrKey::Url(url.clone()),
                crate::pr_merged_wakes::CompletionObservation::Branch(cwd, branch) => PrKey::Branch { cwd: cwd.clone(), branch: branch.clone() },
                crate::pr_merged_wakes::CompletionObservation::BranchUnknown(..) => continue,
            };
            if pending.len() < 512 && !cache.get(&key).is_some_and(Entry::fresh_pr) { pending.insert(key); }
        }
    }

    pub(crate) fn set_merged_wakes(&self, wakes: crate::pr_merged_wakes::PrMergedWakes) {
        *lock(&self.inner.merged_wakes) = Some(wakes);
    }

    /// Attach the Linear API client (engine assembly).
    pub fn set_linear(
        &self,
        linear: crate::linear::LinearService,
        workspace: crate::workspace_host::WorkspaceHost,
    ) {
        *lock(&self.inner.linear) = Some((linear, workspace));
    }

    /// Discard ticket results for a changed credential, including in-flight reads.
    pub fn invalidate_linear_account(&self, account_id: &str) {
        let account = (account_id != "default").then(|| account_id.to_owned());
        let mut revisions = lock(&self.inner.linear_ticket_revisions);
        *revisions.entry(account.clone()).or_default() += 1;
        lock(&self.inner.ticket_cache).retain(|key, _| {
            !matches!(key, TicketKey::Linear { account_id, .. } if *account_id == account)
        });
    }

    /// [`Self::status_for_links`] for a chat in `space_id`: when that Space
    /// has Linear on, its ticket is fetched through the Linear API (if a
    /// key is set) instead of the `linear` CLI.
    pub fn status_for_chat_links(
        &self,
        space_id: Option<&str>,
        cwd: Option<&str>,
        branch: Option<&str>,
        linked_pr_links: &[mast_proto::ChatPrLink],
        linked_ticket_id: Option<&str>,
    ) -> ChatLinkStatus {
        self.status_for_chat_links_with_checkout(
            space_id,
            cwd,
            branch,
            linked_pr_links,
            linked_ticket_id,
            None,
        )
    }

    /// Resolve the PR from a selected checkout while retaining the chat's
    /// original ticket and execution context.
    pub fn status_for_chat_links_with_checkout(
        &self,
        space_id: Option<&str>,
        cwd: Option<&str>,
        branch: Option<&str>,
        linked_pr_links: &[mast_proto::ChatPrLink],
        linked_ticket_id: Option<&str>,
        checkout: Option<(&str, Option<&str>)>,
    ) -> ChatLinkStatus {
        let account = space_id.and_then(|space| {
            lock(&self.inner.linear)
                .as_ref()
                .and_then(|(linear, workspace)| workspace.space_linear_account(space).map(|account| {
                    let account = account.filter(|id| id != "default");
                    let revision = linear.for_account(account.as_deref()).credential_revision();
                    (account, revision)
                }))
        });
        self.status_for_links_with_checkout(
            cwd,
            branch,
            linked_pr_links,
            linked_ticket_id,
            checkout,
            account,
        )
    }

    /// Fetch an account-scoped ticket through Linear, or an unscoped one
    /// through the CLI. An unavailable account never falls back to another.
    async fn fetch_ticket(&self, key: TicketKey) -> Option<TicketStatus> {
        match key {
            TicketKey::Linear { identifier, account_id, credential_revision } => {
                let linear = lock(&self.inner.linear).as_ref().map(|(l, _)| l.clone())?;
                let linear = linear.for_account(account_id.as_deref());
                if linear.credential_revision() != credential_revision { return None; }
                linear.issue(&identifier, false)
                    .await
                    .ok()
                    .map(|issue| TicketStatus {
                        identifier: issue.identifier,
                        title: Some(issue.title),
                        url: Some(issue.url),
                        status: Some(issue.state.name),
                    })
            }
            TicketKey::Cli(identifier) => tokio::task::spawn_blocking(move || fetch_linear_ticket(&identifier))
                .await
                .unwrap_or(None),
        }
    }

    /// Pure cache read, never blocks on a live `gh`/`linear` call. A branch
    /// with nothing cached yet is registered for the next sweep tick and
    /// reads back as `None` for that piece until then.
    ///
    /// Lookup order (ticket 0xx): `linked_pr_url`/`linked_ticket_id` — the
    /// chat row's durable link — wins over the branch-based `gh pr view`
    /// guess / ticket-id regex when present; a link-less chat (`None` for
    /// both) keeps EXACTLY the pre-existing branch/regex behavior. Callers
    /// (`CHAT_LINK_STATUS`'s handler) attach `prSource`/`ticketSource`
    /// themselves from the chat row — this method only decides WHICH key to
    /// resolve, not the provenance to report.
    pub fn status_for(
        &self,
        cwd: Option<&str>,
        branch: Option<&str>,
        linked_pr_url: Option<&str>,
        linked_ticket_id: Option<&str>,
    ) -> ChatLinkStatus {
        let legacy_link =
            linked_pr_url
                .filter(|url| !url.is_empty())
                .map(|url| mast_proto::ChatPrLink {
                    url: url.to_string(),
                    // `status_for` predates per-link provenance; callers attach
                    // the real legacy source to `pr_source` after this read.
                    source: mast_proto::ChatLinkSource::Mentioned,
                });
        self.status_for_links(cwd, branch, legacy_link.as_slice(), linked_ticket_id)
    }

    /// Multi-PR sibling of [`Self::status_for`]. Every durable URL gets its
    /// own cache key and stable wire row, including while detail is pending.
    /// The branch-derived PR is also retained when it resolves to a different
    /// URL, so manually adding one PR does not hide a stacked/sibling PR that
    /// the checkout itself identifies.
    pub fn status_for_links(
        &self,
        cwd: Option<&str>,
        branch: Option<&str>,
        linked_pr_links: &[mast_proto::ChatPrLink],
        linked_ticket_id: Option<&str>,
    ) -> ChatLinkStatus {
        self.status_for_links_with_checkout(cwd, branch, linked_pr_links, linked_ticket_id, None, None)
    }

    fn status_for_links_with_checkout(
        &self,
        cwd: Option<&str>,
        branch: Option<&str>,
        linked_pr_links: &[mast_proto::ChatPrLink],
        linked_ticket_id: Option<&str>,
        checkout: Option<(&str, Option<&str>)>,
        linear_account: Option<(Option<String>, u64)>,
    ) -> ChatLinkStatus {
        let mut status = ChatLinkStatus::default();
        let automatic_cwd =
            cwd.filter(|cwd| !crate::repos::is_automatic_access_blocked(std::path::Path::new(cwd)));

        for link in linked_pr_links.iter().filter(|link| !link.url.is_empty()) {
            let key = PrKey::Url(link.url.clone());
            let cache = lock(&self.inner.pr_cache);
            let detail = match cache.get(&key) {
                Some(entry) if entry.fresh_pr() => entry.data.clone(),
                // Expired: keep showing the last known state while the
                // sweep refreshes it, so a stage never falls back to "unknown".
                Some(entry) if entry.servable_stale() => {
                    let data = entry.data.clone();
                    drop(cache);
                    self.queue_pr_refresh(key);
                    data
                }
                _ => {
                    drop(cache);
                    self.queue_pr_refresh(key);
                    None
                }
            };
            status.pr_links.push(PrLinkStatus {
                url: link.url.clone(),
                source: Some(link.source),
                detail,
            });
        }

        // Preserve the historical primary fields for old clients.
        if let Some(primary) = status.pr_links.first() {
            status.pr = primary.detail.clone();
            status.pr_source = primary.source;
        }

        // Branch inference remains useful even when durable links exist (for
        // stacked PRs), but linked URLs are authoritative: do not enqueue a
        // redundant branch lookup just because its cache entry is absent.
        // Already-cached branch detail may still add a distinct inferred PR,
        // and URL de-duplication gives durable provenance precedence.
        let has_durable_pr_links = !status.pr_links.is_empty();
        let (pr_cwd, pr_branch) = checkout.map_or((automatic_cwd, branch), |(path, branch)| {
            (
                Some(path).filter(|cwd| {
                    !crate::repos::is_automatic_access_blocked(std::path::Path::new(cwd))
                }),
                branch,
            )
        });
        let branch_key = match (pr_cwd, pr_branch) {
            (Some(cwd), Some(branch)) if !branch.is_empty() => {
                let cwd = PathBuf::from(cwd);
                let branch = branch.to_string();
                Some(if checkout.is_some() {
                    PrKey::CheckoutBranch { cwd, branch }
                } else {
                    PrKey::Branch { cwd, branch }
                })
            }
            _ => None,
        };
        if let Some(key) = branch_key {
            let mut refresh = false;
            {
                let cache = lock(&self.inner.pr_cache);
                match cache.get(&key) {
                    Some(entry) if entry.fresh_pr() || entry.servable_stale() => {
                        // An expired inferred PR is still shown, but always re-checked
                        // (a stacked PR may have merged), even with durable links.
                        refresh = !entry.fresh_pr();
                        if let Some(detail) = entry.data.clone() {
                            let inferred_url = detail.url.clone().unwrap_or_default();
                            if !inferred_url.is_empty()
                                && !status.pr_links.iter().any(|link| link.url == inferred_url)
                            {
                                status.pr_links.push(PrLinkStatus {
                                    url: inferred_url,
                                    source: None,
                                    detail: Some(detail.clone()),
                                });
                                if status.pr.is_none() {
                                    status.pr = Some(detail);
                                }
                            } else if status.pr.is_none() {
                                status.pr = Some(detail);
                            }
                        }
                    }
                    _ => refresh = !has_durable_pr_links,
                }
            }
            if refresh { self.queue_pr_refresh(key); }
        }

        let ticket_id = linked_ticket_id
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| branch.and_then(extract_ticket_id));
        if let Some(ticket_id) = ticket_id {
            let ticket_id = match linear_account {
                Some((account_id, credential_revision)) => TicketKey::Linear { identifier: ticket_id, account_id, credential_revision },
                None => TicketKey::Cli(ticket_id),
            };
            let cache = lock(&self.inner.ticket_cache);
            match cache.get(&ticket_id) {
                Some(entry) if entry.fresh(CACHE_TTL) => status.ticket = entry.data.clone(),
                _ => {
                    drop(cache);
                    lock(&self.inner.pending_ticket).insert(ticket_id);
                }
            }
        }

        // Worktree/diff-stat: pure string check + a local, cheap-but-not-free
        // `git` shell-out gated the same way PR/ticket are — never inline, a
        // cache read that registers a miss for the sweep to fill.
        let context_cwd = checkout.map(|(path, _)| path).or(cwd);
        if let Some(cwd) = context_cwd {
            status.is_worktree =
                !crate::repos::is_automatic_access_blocked(std::path::Path::new(cwd))
                    && is_worktree_cwd(cwd);
            if status.is_worktree {
                let key = PathBuf::from(cwd);
                let cache = lock(&self.inner.diff_stat_cache);
                match cache.get(&key) {
                    Some(entry) if entry.fresh(DIFF_STAT_TTL) => status.diff_stat = entry.data,
                    _ => {
                        drop(cache);
                        lock(&self.inner.pending_diff_stat).insert(key);
                    }
                }
            }
        }

        status
    }

    /// Pure cache read: every PR currently in the last-fetched list, paired
    /// with whatever detail the throttled sweep has filled in so far (`None`
    /// until then). Empty `Vec` before the first sweep tick populates the
    /// list, or if `gh search prs` found nothing / isn't authenticated.
    pub fn my_open_prs(&self) -> Vec<MyPrItem> {
        let list = lock(&self.inner.my_prs_list)
            .as_ref()
            .map(|(items, _)| items.clone())
            .unwrap_or_default();
        let details = lock(&self.inner.my_pr_detail);
        list.into_iter()
            .map(|summary| {
                let detail = summary
                    .repo
                    .as_ref()
                    .and_then(|repo| details.get(&(repo.clone(), summary.number)))
                    .and_then(|entry| entry.data.clone());
                MyPrItem { summary, detail }
            })
            .collect()
    }

    /// Wait at most `timeout` for the first authored-PR search to finish.
    /// Returns `true` after either a successful list fill or a recorded
    /// failure. Once ready, this is an immediate cache-state check.
    pub async fn wait_for_my_open_prs_ready(&self, timeout: Duration) -> bool {
        let mut ready = self.inner.my_prs_ready.subscribe();
        if *ready.borrow_and_update() {
            return true;
        }
        tokio::time::timeout(timeout, ready.wait_for(|ready| *ready))
            .await
            .is_ok()
    }

    /// Pure cache read: open PRs where review is currently requested from the
    /// signed-in user. `detail` is always `None` — the detail sweep only
    /// covers the authored list. No reviewed-by exclusion on purpose: GitHub
    /// drops you from `review-requested` once you submit a review, and a
    /// re-request must stay visible.
    pub fn review_requested_prs(&self) -> Vec<MyPrItem> {
        lock(&self.inner.review_prs_list)
            .as_ref()
            .map(|(items, _)| items.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|summary| MyPrItem {
                summary,
                detail: None,
            })
            .collect()
    }

    /// Spawn the throttled background sweep. Caller keeps the returned
    /// handle alive for as long as the sweep should run (dropping it aborts
    /// the loop) — `EngineCore` holds it for its own lifetime, same as any
    /// other engine-owned background task.
    pub fn spawn_sweep_loop(&self) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(SWEEP_INTERVAL);
            loop {
                interval.tick().await;
                this.sweep_once().await;
            }
        })
    }

    async fn sweep_once(&self) {
        let scan_due = {
            let mut scan = lock(&self.inner.completion_scan);
            if scan.0.is_none_or(|last| last.elapsed() >= COMPLETION_SCAN_INTERVAL) {
                scan.0 = Some(Instant::now());
                true
            } else { false }
        };
        if scan_due {
            let wakes = lock(&self.inner.merged_wakes).clone();
            if let Some(wakes) = wakes {
                let cache = self.clone();
                match tokio::task::spawn_blocking(move || {
                    let observations = wakes.completion_observations()?;
                    Ok::<_, String>(wakes.resolve_completion_observations(cache.select_completion_observations(&observations)))
                }).await {
                    Ok(Ok(observations)) => self.queue_completion_observations(&observations),
                    Ok(Err(error)) => tracing::warn!(%error, "failed to collect merged PR observations"),
                    Err(error) => tracing::warn!(%error, "merged PR observation collection panicked"),
                }
            }
        }
        let pr_keys = self.take_pending_prs();
        let mut pr_results = Vec::with_capacity(pr_keys.len());
        for key in pr_keys {
            let inner = self.inner.clone();
            let key2 = key.clone();
            let data = tokio::task::spawn_blocking(move || match &key2 {
                PrKey::Branch { cwd, branch } => fetch_github_pr(cwd, branch),
                PrKey::CheckoutBranch { cwd, branch } => match run_gh_pr_view(cwd, branch) {
                    GhView::Found(data) => PrLookup::Found(pr_json_to_status(&data)),
                    GhView::NotFound => PrLookup::NotFound,
                    GhView::Error => PrLookup::Error,
                },
                PrKey::Url(url) => fetch_github_pr_by_url(url),
            })
            .await
            .unwrap_or(PrLookup::Error);
            let wakes = lock(&inner.merged_wakes).clone();
            if let (Some(wakes), PrLookup::Found(status)) = (&wakes, &data) {
                let url = match &key {
                    PrKey::Url(url) => Some(url.as_str()),
                    PrKey::Branch { .. } | PrKey::CheckoutBranch { .. } => status.url.as_deref(),
                };
                if let Some(url) = url {
                    let checkout = match &key {
                        PrKey::Branch { cwd, branch } | PrKey::CheckoutBranch { cwd, branch } => Some((cwd.as_path(), branch.as_str())),
                        PrKey::Url(_) => None,
                    };
                    wakes.observe_with_checkout(url, status, checkout).await;
                }
            }
            pr_results.push((key, data));
        }
        self.publish_pr_results(pr_results);

        let ticket_ids: Vec<TicketKey> = {
            let mut pending = lock(&self.inner.pending_ticket);
            let take: Vec<TicketKey> = pending.iter().take(KEYS_PER_SWEEP).cloned().collect();
            for id in &take {
                pending.remove(id);
            }
            take
        };
        for id in ticket_ids {
            let inner = self.inner.clone();
            let revision = match &id {
                TicketKey::Linear { account_id, .. } => Some(*lock(&inner.linear_ticket_revisions).get(account_id).unwrap_or(&0)),
                TicketKey::Cli(_) => None,
            };
            let data = self.fetch_ticket(id.clone()).await;
            let revisions = lock(&inner.linear_ticket_revisions);
            let current = match &id {
                TicketKey::Linear { account_id, .. } => Some(*revisions.get(account_id).unwrap_or(&0)),
                TicketKey::Cli(_) => None,
            };
            if revision == current {
                lock(&inner.ticket_cache).insert(id, Entry { data, fetched_at: Instant::now() });
            } else {
                lock(&inner.pending_ticket).insert(id);
            }
        }

        let diff_stat_keys: Vec<PathBuf> = {
            let mut pending = lock(&self.inner.pending_diff_stat);
            let take: Vec<PathBuf> = pending.iter().take(KEYS_PER_SWEEP).cloned().collect();
            for key in &take {
                pending.remove(key);
            }
            take
        };
        for cwd in diff_stat_keys {
            let inner = self.inner.clone();
            let cwd2 = cwd.clone();
            let data = tokio::task::spawn_blocking(move || git_diff_shortstat(&cwd2))
                .await
                .unwrap_or(None);
            lock(&inner.diff_stat_cache).insert(
                cwd,
                Entry {
                    data,
                    fetched_at: Instant::now(),
                },
            );
        }

        // Never run both `gh search prs` calls on the same tick: if the
        // authored search fired, the review-requested one waits for the next
        // tick (the two TTLs then drift apart and stay interleaved).
        if !self.sweep_my_prs_list().await {
            self.sweep_review_requested_list().await;
        }
        self.sweep_one_my_pr_detail().await;
    }

    /// Drain up to `KEYS_PER_SWEEP` pending PR keys, oldest observation first
    /// (never-fetched keys first of all).
    fn take_pending_prs(&self) -> Vec<PrKey> {
        // One lock at a time: `queue_completion_observations` takes the cache
        // before pending, so never hold pending while reading the cache.
        let keys: Vec<PrKey> = lock(&self.inner.pending_pr).iter().cloned().collect();
        let mut all: Vec<(Option<Instant>, PrKey)> = {
            let cache = lock(&self.inner.pr_cache);
            keys.into_iter().map(|key| (cache.get(&key).map(|entry| entry.fetched_at), key)).collect()
        };
        all.sort_by_key(|(at, _)| *at);
        let take: Vec<PrKey> = all.into_iter().take(KEYS_PER_SWEEP).map(|(_, key)| key).collect();
        let mut pending = lock(&self.inner.pending_pr);
        for key in &take { pending.remove(key); }
        take
    }

    /// Store one batch. The revision moves only when some cached PR state
    /// actually changed; the update names the changed URLs.
    fn publish_pr_results(&self, results: Vec<(PrKey, PrLookup)>) {
        if results.is_empty() { return; }
        let mut urls = HashSet::<String>::new();
        let mut branches = Vec::<PrBranchKey>::new();
        let mut cache = lock(&self.inner.pr_cache);
        let mut retry = lock(&self.inner.pr_retry);
        for (key, lookup) in results {
            let new = match lookup {
                PrLookup::Error => {
                    // Keep the entry (and its fetched_at): unverified data must not look fresh.
                    let attempts = retry.get(&key).map_or(0, |(attempts, _)| *attempts);
                    let delay = mast_harness::backoff::jittered(mast_harness::backoff::delay_for_attempt(PR_RETRY_BASE, PR_RETRY_CEILING, attempts));
                    retry.insert(key, (attempts.saturating_add(1), Instant::now() + delay));
                    continue;
                }
                PrLookup::Found(status) => Some(status),
                PrLookup::NotFound => None,
            };
            retry.remove(&key);
            let old = cache.get(&key).and_then(|entry| entry.data.clone());
            if old != new {
                for status in [&old, &new].into_iter().flatten() {
                    if let Some(url) = &status.url { urls.insert(url.clone()); }
                }
                match &key {
                    PrKey::Url(url) => { urls.insert(url.clone()); }
                    PrKey::Branch { cwd, branch } | PrKey::CheckoutBranch { cwd, branch } => {
                        let key = PrBranchKey { cwd: cwd.to_string_lossy().into_owned(), branch: branch.clone() };
                        if !branches.contains(&key) { branches.push(key); }
                    }
                }
            }
            cache.insert(key, Entry { data: new, fetched_at: Instant::now() });
        }
        drop(retry);
        drop(cache);
        if urls.is_empty() && branches.is_empty() { return; }
        let mut urls: Vec<String> = urls.into_iter().collect();
        urls.sort();
        self.inner.pr_revision.send_modify(|update| {
            update.revision = update.revision.wrapping_add(1);
            update.urls = urls;
            update.branches = branches;
        });
    }

    /// Refresh the "my open PRs" list if stale. Own TTL/cadence, separate
    /// from the per-branch PR/ticket sweep above — matches
    /// `my_prs_refresh_loop`'s comment that the list call is cheap enough to
    /// redo on its own schedule without competing with the hot path.
    ///
    /// Returns `true` if a search was actually attempted this tick.
    async fn sweep_my_prs_list(&self) -> bool {
        self.sweep_my_prs_list_with(fetch_my_open_prs).await
    }

    async fn sweep_my_prs_list_with(&self, fetch: fn() -> Option<Vec<MyOpenPr>>) -> bool {
        let attempted =
            sweep_search_slot(&self.inner.my_prs_list, &self.inner.my_prs_failed_at, fetch).await;
        if attempted {
            self.inner.my_prs_ready.send_replace(true);
        }
        attempted
    }

    /// Same as `sweep_my_prs_list` for the review-requested search.
    async fn sweep_review_requested_list(&self) -> bool {
        sweep_search_slot(
            &self.inner.review_prs_list,
            &self.inner.review_prs_failed_at,
            fetch_review_requested_prs,
        )
        .await
    }

    /// Fill in ONE stale/missing PR detail per tick — "stalest first" per
    /// `my_prs_refresh_loop`'s one-per-tick detail throttling, keyed by
    /// `(repo, number)` rather than `(cwd, branch)` since the list call never
    /// gives us a local checkout path.
    async fn sweep_one_my_pr_detail(&self) {
        let needed: Vec<(String, u64)> = lock(&self.inner.my_prs_list)
            .as_ref()
            .map(|(items, _)| {
                items
                    .iter()
                    .filter_map(|pr| pr.repo.clone().map(|repo| (repo, pr.number)))
                    .collect()
            })
            .unwrap_or_default();
        let target = {
            let details = lock(&self.inner.my_pr_detail);
            needed
                .into_iter()
                .find(|key| !details.get(key).is_some_and(|entry| entry.fresh(CACHE_TTL)))
        };
        let Some((repo, number)) = target else {
            return;
        };
        let (repo2, number2) = (repo.clone(), number);
        let data = tokio::task::spawn_blocking(move || fetch_github_pr_by_number(&repo2, number2))
            .await
            .unwrap_or(None);
        lock(&self.inner.my_pr_detail).insert(
            (repo, number),
            Entry {
                data,
                fetched_at: Instant::now(),
            },
        );
    }
}

impl Default for PrTicketCache {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `extract_ticket_id` (`session_canvas_server.py:1363`): the first
/// `[A-Za-z]{2,6}-\d+` "word" in `text` (bounded by non-alphanumeric on both
/// sides, matching the reference's `\b...\b` regex), uppercased. Hand-rolled
/// rather than pulling in the `regex` crate — not a workspace dependency
/// today and this pattern is simple enough not to need one (same call
/// `liveness.rs` made about `sysinfo`).
pub(crate) fn extract_ticket_id(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() {
            let start = i;
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                j += 1;
            }
            let letters = j - start;
            if (2..=6).contains(&letters) && j < bytes.len() && bytes[j] == b'-' {
                let dash = j;
                let mut k = dash + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                let digits = k - (dash + 1);
                let left_ok = start == 0 || !is_word(bytes[start - 1]);
                let right_ok = k >= bytes.len() || !is_word(bytes[k]);
                if digits > 0 && left_ok && right_ok {
                    return Some(text[start..k].to_ascii_uppercase());
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

pub(crate) fn summarize_checks(checks: Option<&Value>) -> Option<ChecksStatus> {
    let checks = checks?.as_array()?;
    if checks.is_empty() {
        return None;
    }
    let mut any_failing = false;
    let mut any_pending = false;
    for c in checks {
        let status = c
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_uppercase();
        let conclusion = c
            .get("conclusion")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_uppercase();
        if !status.is_empty() && status != "COMPLETED" {
            any_pending = true;
        } else if matches!(
            conclusion.as_str(),
            "FAILURE" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE"
        ) {
            any_failing = true;
        } else if matches!(conclusion.as_str(), "SUCCESS" | "NEUTRAL" | "SKIPPED") {
            // passing; no flag needed
        } else {
            any_pending = true;
        }
    }
    Some(if any_failing {
        ChecksStatus::Failing
    } else if any_pending {
        ChecksStatus::Pending
    } else {
        ChecksStatus::Passing
    })
}

/// Bot/self-review filtering, matching `is_bot_login` (`session_canvas_server.py:1370`).
pub(crate) fn is_bot_login(login: &str) -> bool {
    let lower = login.to_lowercase();
    lower.contains("bot") || lower.contains("github-actions") || lower.contains("greptile")
}

fn pr_json_to_status(data: &Value) -> PrStatus {
    let reviews = data
        .get("reviews")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let author_login = data
        .get("author")
        .and_then(|a| a.get("login"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut reviewers: Vec<String> = reviews
        .iter()
        .filter_map(|r| {
            r.get("author")
                .and_then(|a| a.get("login"))
                .and_then(Value::as_str)
        })
        .filter(|login| !is_bot_login(login) && Some(login.to_string()) != author_login)
        .map(str::to_string)
        .collect();
    reviewers.sort();
    reviewers.dedup();

    PrStatus {
        additions: data.get("additions").and_then(Value::as_u64),
        deletions: data.get("deletions").and_then(Value::as_u64),
        merged_at: data.get("mergedAt").and_then(Value::as_str).map(str::to_owned),
        number: data.get("number").and_then(Value::as_u64).unwrap_or(0),
        url: data.get("url").and_then(Value::as_str).map(str::to_string),
        state: data
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_lowercase(),
        is_draft: data
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        review_decision: data
            .get("reviewDecision")
            .and_then(Value::as_str)
            .map(str::to_string),
        reviewers,
        checks: summarize_checks(data.get("statusCheckRollup")),
        title: data
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
        branch: data
            .get("headRefName")
            .and_then(Value::as_str)
            .map(str::to_string),
        head_sha: data
            .get("headRefOid")
            .and_then(Value::as_str)
            .filter(|sha| !sha.is_empty())
            .map(str::to_string),
        mergeable: data
            .get("mergeable")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_lowercase(),
        has_reviewer_requested: data
            .get("reviewRequests")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty()),
    }
}

/// `.gitmodules` submodule paths under `root_cwd`, matching
/// `workspace_submodule_paths` (`session_canvas_server.py:1465`) — read
/// directly rather than hardcoding any repo's own submodule list, so it
/// keeps working if that list changes.
pub(crate) fn workspace_submodule_paths(root_cwd: &Path) -> Vec<String> {
    let gitmodules = root_cwd.join(".gitmodules");
    let Ok(contents) = std::fs::read_to_string(&gitmodules) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("path").and_then(|rest| {
                rest.trim_start()
                    .strip_prefix('=')
                    .map(|v| v.trim().to_string())
            })
        })
        .collect()
}

/// Outcome of one `gh pr view`.
enum GhView {
    Found(Value),
    NotFound,
    Error,
}

/// Tell "this PR does not exist" apart from a transient failure by gh's own words.
fn gh_view_not_found(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    ["no pull requests found", "no pull request found", "could not resolve to a pullrequest", "http 404"]
        .iter().any(|needle| stderr.contains(needle))
}

fn gh_view_result(output: std::io::Result<std::process::Output>) -> GhView {
    let Ok(output) = output else { return GhView::Error; };
    if output.status.success() {
        return serde_json::from_slice(&output.stdout).map_or(GhView::Error, GhView::Found);
    }
    if gh_view_not_found(&String::from_utf8_lossy(&output.stderr)) { GhView::NotFound } else { GhView::Error }
}

fn run_gh_pr_view(cwd: &Path, branch: &str) -> GhView {
    let Some(mut command) = gh_command() else { return GhView::Error; };
    gh_view_result(command.args(["pr", "view", branch, "--json", PR_VIEW_JSON_FIELDS]).current_dir(cwd).output())
}

/// `fetch_github_pr` (`session_canvas_server.py:1505`): try `cwd` directly,
/// then each `.gitmodules` submodule under it — confirmed in the reference
/// implementation's own comments that a monorepo superproject root often has
/// no PR for a branch that has a real, open one exactly one submodule down.
fn fetch_github_pr(cwd: &Path, branch: &str) -> PrLookup {
    let mut errored = false;
    match run_gh_pr_view(cwd, branch) {
        GhView::Found(data) => return PrLookup::Found(pr_json_to_status(&data)),
        GhView::Error => return PrLookup::Error,
        GhView::NotFound => {}
    }
    for sub_cwd in workspace_submodule_paths(cwd).into_iter().map(|sub| cwd.join(sub)).filter(|sub_cwd| sub_cwd.is_dir()) {
        match run_gh_pr_view(&sub_cwd, branch) {
            GhView::Found(data) => return PrLookup::Found(pr_json_to_status(&data)),
            GhView::Error => errored = true,
            GhView::NotFound => {}
        }
    }
    if errored { PrLookup::Error } else { PrLookup::NotFound }
}

/// PR detail resolved from a durable `linked_pr_url` (ticket 0xx) rather than
/// a branch guess: `gh pr view <url>` is fully self-describing, so — unlike
/// [`fetch_github_pr`] — this needs no local checkout/cwd at all (the process
/// still runs from SOME cwd, but `gh` resolves the target repo from the URL,
/// not from where it happens to be invoked).
fn fetch_github_pr_by_url(url: &str) -> PrLookup {
    let Some(mut command) = gh_command() else { return PrLookup::Error; };
    match gh_view_result(command.args(["pr", "view", url, "--json", PR_VIEW_JSON_FIELDS]).output()) {
        GhView::Found(data) => PrLookup::Found(pr_json_to_status(&data)),
        GhView::NotFound => PrLookup::NotFound,
        GhView::Error => PrLookup::Error,
    }
}

/// `git_diff_stat` (`session_canvas_server.py:312-338`): uncommitted changes
/// only, via `git diff --shortstat HEAD` run with `cwd` as the working
/// directory. `None` when `cwd` isn't a git repo, has no `HEAD` yet (a
/// brand-new worktree before its first commit), or the command otherwise
/// fails — same "nothing to show yet" contract as every other fetch in this
/// module; the caller doesn't distinguish "not a repo" from "clean" (a
/// genuinely clean worktree instead reads as `Some(DiffStat::default())`,
/// since `git diff --shortstat` succeeds with empty stdout in that case).
fn git_diff_shortstat(cwd: &Path) -> Option<DiffStat> {
    let output = mast_harness::git_command()
        .args(["diff", "--shortstat", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_git_shortstat(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// `git diff --shortstat`'s one-line summary, e.g.
/// `" 3 files changed, 10 insertions(+), 4 deletions(-)"` — any subset of the
/// three clauses can be absent (a pure-deletion diff has no "insertions"
/// clause at all, and vice versa), and a clean tree produces an empty string
/// entirely (`DiffStat::default()`, all zero). Hand-parsed rather than
/// pulling in the `regex` crate, matching `extract_ticket_id`'s precedent
/// just above.
fn parse_git_shortstat(text: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    for clause in text.trim().split(',') {
        let clause = clause.trim();
        let Some(space_idx) = clause.find(char::is_whitespace) else {
            continue;
        };
        let (number, rest) = clause.split_at(space_idx);
        let Ok(number) = number.parse::<u32>() else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.starts_with("file") {
            stat.files_changed = number;
        } else if rest.starts_with("insertion") {
            stat.lines_added = number;
        } else if rest.starts_with("deletion") {
            stat.lines_removed = number;
        }
    }
    stat
}

/// `fetch_my_open_prs` (`session_canvas_server.py:1554`), deliberately
/// WITHOUT that function's `--owner` scoping: the reference hardcodes
/// `MY_PRS_OWNER = "Cofactr"` (one specific org), which only makes sense for
/// its one deployment. `--author=@me --state=open` alone, with no `--owner`,
/// searches every repo the signed-in `gh` account can see — the general
/// per-user equivalent that works for anyone, not just this org.
fn fetch_my_open_prs() -> Option<Vec<MyOpenPr>> {
    run_pr_search(
        "--author=@me",
        "number,title,url,repository,isDraft,updatedAt",
    )
}

/// Open PRs where the signed-in user is a requested reviewer. Deliberately NO
/// `-reviewed-by:@me` exclusion: GitHub already drops you from
/// `review-requested` once you submit a review, and a re-request (which must
/// stay visible) would be hidden by that qualifier.
fn fetch_review_requested_prs() -> Option<Vec<MyOpenPr>> {
    run_pr_search(
        "--review-requested=@me",
        "number,title,url,repository,isDraft,updatedAt,author",
    )
}

fn run_pr_search(qualifier: &str, json_fields: &str) -> Option<Vec<MyOpenPr>> {
    let output = gh_command()?
        .args([
            "search",
            "prs",
            qualifier,
            "--state=open",
            "--json",
            json_fields,
            "--limit",
            "50",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_pr_search(&output.stdout)
}

pub(crate) fn parse_pr_search(stdout: &[u8]) -> Option<Vec<MyOpenPr>> {
    let data: Vec<Value> = serde_json::from_slice(stdout).ok()?;
    Some(
        data.iter()
            .map(|d| MyOpenPr {
                number: d.get("number").and_then(Value::as_u64).unwrap_or(0),
                title: d.get("title").and_then(Value::as_str).map(str::to_string),
                url: d.get("url").and_then(Value::as_str).map(str::to_string),
                repo: d
                    .get("repository")
                    .and_then(|r| r.get("nameWithOwner"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                is_draft: d.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
                updated_at: d
                    .get("updatedAt")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                author: d
                    .get("author")
                    .and_then(|a| a.get("login"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect(),
    )
}

/// Is a search slot due for a (re)fetch? Stale-or-empty list AND not inside
/// the post-failure backoff window.
fn search_due(list_fetched_at: Option<Instant>, failed_at: Option<Instant>) -> bool {
    if failed_at.is_some_and(|t| t.elapsed() < SEARCH_FAILURE_BACKOFF) {
        return false;
    }
    list_fetched_at.is_none_or(|t| t.elapsed() >= MY_PRS_LIST_TTL)
}

/// Shared TTL + failure-backoff refresh for one `gh search prs` slot.
/// Returns `true` if a search was attempted.
async fn sweep_search_slot(
    list: &Mutex<Option<(Vec<MyOpenPr>, Instant)>>,
    failed_at: &Mutex<Option<Instant>>,
    fetch: fn() -> Option<Vec<MyOpenPr>>,
) -> bool {
    let due = search_due(lock(list).as_ref().map(|(_, at)| *at), *lock(failed_at));
    if !due {
        return false;
    }
    match tokio::task::spawn_blocking(fetch).await.unwrap_or(None) {
        Some(items) => {
            *lock(list) = Some((items, Instant::now()));
            *lock(failed_at) = None;
        }
        None => *lock(failed_at) = Some(Instant::now()),
    }
    true
}

/// `fetch_github_pr_by_number`: detail fetch keyed by `(repo, number)`
/// instead of `(cwd, branch)` — `gh pr view <number> --repo <owner/repo>`
/// works from anywhere, no local checkout needed, matching how the list
/// call itself has no cwd to begin with.
fn fetch_github_pr_by_number(repo: &str, number: u64) -> Option<PrStatus> {
    let output = gh_command()?
        .args([
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            repo,
            "--json",
            PR_VIEW_JSON_FIELDS,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let data: Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(pr_json_to_status(&data))
}

/// Every GitHub CLI call shares the same non-interactive, GUI-safe process
/// setup. In particular, app launches do not inherit the login shell PATH
/// that commonly contains Homebrew's `gh`.
fn gh_command() -> Option<Command> {
    let mut command = mast_harness::gh_command()?;
    command.env("GH_PROMPT_DISABLED", "1").stdin(Stdio::null());
    Some(command)
}

/// `fetch_linear_ticket` (`session_canvas_server.py:1595`). `linear` may not
/// be installed (confirmed absent on this machine) — that's a normal `None`,
/// not an error; every failure mode here (missing binary, non-zero exit,
/// unparseable output) collapses to the same "no ticket data available".
fn fetch_linear_ticket(ticket_id: &str) -> Option<TicketStatus> {
    let output = mast_harness::command("linear")
        .args(["issue", "view", ticket_id, "--json"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let data: Value = serde_json::from_slice(&output.stdout).ok()?;
    let status = data
        .get("state")
        .and_then(|s| s.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(TicketStatus {
        identifier: data
            .get("identifier")
            .and_then(Value::as_str)
            .unwrap_or(ticket_id)
            .to_string(),
        title: data
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string),
        url: data.get("url").and_then(Value::as_str).map(str::to_string),
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ticket_id_from_branch_names() {
        assert_eq!(
            extract_ticket_id("eng-2715-fix-thing"),
            Some("ENG-2715".into())
        );
        assert_eq!(
            extract_ticket_id("feature/ABC-123-do-stuff"),
            Some("ABC-123".into())
        );
        assert_eq!(extract_ticket_id("mikey/quick-fix"), None);
        assert_eq!(extract_ticket_id(""), None);
        // 7-letter prefix is outside the `{2,6}` bound — must not match.
        assert_eq!(extract_ticket_id("abcdefg-123"), None);
        // No digits after the dash — must not match.
        assert_eq!(extract_ticket_id("eng-"), None);
    }

    #[test]
    fn extract_ticket_id_requires_word_boundary() {
        // "xeng-123x" must not match "ENG-123" — the char before/after the
        // whole token must not itself be alphanumeric.
        assert_eq!(extract_ticket_id("xeng-123x"), None);
        assert_eq!(
            extract_ticket_id("prefix-eng-123-suffix"),
            Some("ENG-123".into())
        );
    }

    #[test]
    fn checks_summary_prioritizes_failing_over_pending_over_passing() {
        let failing = serde_json::json!([
            {"status": "COMPLETED", "conclusion": "SUCCESS"},
            {"status": "COMPLETED", "conclusion": "FAILURE"},
        ]);
        assert_eq!(
            summarize_checks(Some(&failing)),
            Some(ChecksStatus::Failing)
        );

        let pending = serde_json::json!([
            {"status": "COMPLETED", "conclusion": "SUCCESS"},
            {"status": "IN_PROGRESS", "conclusion": null},
        ]);
        assert_eq!(
            summarize_checks(Some(&pending)),
            Some(ChecksStatus::Pending)
        );

        let passing = serde_json::json!([{"status": "COMPLETED", "conclusion": "SUCCESS"}]);
        assert_eq!(
            summarize_checks(Some(&passing)),
            Some(ChecksStatus::Passing)
        );

        assert_eq!(summarize_checks(None), None);
    }

    #[test]
    fn pr_json_to_status_filters_bot_and_self_reviews() {
        let data = serde_json::json!({
            "number": 42,
            "url": "https://example.com/pr/42",
            "state": "OPEN",
            "isDraft": false,
            "reviewDecision": "APPROVED",
            "reviews": [
                {"author": {"login": "greptile-apps"}},
                {"author": {"login": "the-author"}},
                {"author": {"login": "real-reviewer"}},
            ],
            "title": "Fix the thing",
            "headRefName": "eng-2715-fix-thing",
            "mergeable": "MERGEABLE",
            "reviewRequests": [],
            "author": {"login": "the-author"},
        });
        let status = pr_json_to_status(&data);
        assert_eq!(status.reviewers, vec!["real-reviewer".to_string()]);
        assert_eq!(status.mergeable, "mergeable");
        assert!(!status.has_reviewer_requested);
    }

    #[test]
    fn pr_status_head_sha_is_optional_and_round_trips() {
        let sha = "0123456789012345678901234567890123456789";
        let status = pr_json_to_status(&serde_json::json!({"headRefOid": sha}));
        assert_eq!(status.head_sha.as_deref(), Some(sha));
        let decoded: PrStatus =
            serde_json::from_value(serde_json::to_value(&status).unwrap()).unwrap();
        assert_eq!(decoded.head_sha.as_deref(), Some(sha));
        assert_eq!(pr_json_to_status(&serde_json::json!({})).head_sha, None);
        assert!(PR_VIEW_JSON_FIELDS.split(',').any(|field| field == "headRefOid"));
    }

    #[test]
    fn pr_status_preserves_optional_change_counts() {
        let status = pr_json_to_status(&serde_json::json!({"additions": 312, "deletions": 48}));
        assert_eq!((status.additions, status.deletions), (Some(312), Some(48)));
        let decoded: PrStatus = serde_json::from_value(serde_json::to_value(status).unwrap()).unwrap();
        assert_eq!((decoded.additions, decoded.deletions), (Some(312), Some(48)));
        let missing = pr_json_to_status(&serde_json::json!({}));
        assert_eq!((missing.additions, missing.deletions), (None, None));
        assert!(PR_VIEW_JSON_FIELDS.split(',').any(|field| field == "additions"));
        assert!(PR_VIEW_JSON_FIELDS.split(',').any(|field| field == "deletions"));
    }

    #[test]
    fn action_reasons_matches_reference_classification() {
        let mut pr = PrStatus {
            additions: None, deletions: None,
            merged_at: None,
            number: 1,
            url: None,
            state: "open".into(),
            is_draft: false,
            review_decision: None,
            reviewers: Vec::new(),
            checks: Some(ChecksStatus::Failing),
            title: None,
            branch: None,
            head_sha: None,
            mergeable: "mergeable".into(),
            has_reviewer_requested: true,
        };
        assert_eq!(pr.action_reasons(), vec![PrAction::CiFailing]);

        pr.checks = Some(ChecksStatus::Passing);
        pr.mergeable = "conflicting".into();
        assert_eq!(pr.action_reasons(), vec![PrAction::MergeConflict]);

        pr.mergeable = "mergeable".into();
        pr.review_decision = Some("CHANGES_REQUESTED".into());
        assert_eq!(pr.action_reasons(), vec![PrAction::ChangesRequested]);

        pr.review_decision = None;
        pr.has_reviewer_requested = false;
        pr.reviewers = Vec::new();
        assert_eq!(pr.action_reasons(), vec![PrAction::NeedsReviewer]);

        pr.has_reviewer_requested = true;
        pr.review_decision = Some("APPROVED".into());
        assert_eq!(pr.action_reasons(), vec![PrAction::ReadyToMerge]);

        pr.is_draft = true;
        assert!(pr.action_reasons().is_empty());
    }

    fn open_pr_for_test(url: &str) -> PrStatus {
        PrStatus {
            additions: None, deletions: None, merged_at: None, number: 7, url: Some(url.into()), state: "OPEN".into(),
            is_draft: false, review_decision: None, reviewers: Vec::new(), checks: None, title: None, branch: None,
            head_sha: None, mergeable: "mergeable".into(), has_reviewer_requested: false,
        }
    }

    #[test]
    fn expired_pr_link_keeps_its_last_known_state_and_queues_a_refresh() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/7";
        lock(&cache.inner.pr_cache).insert(PrKey::Url(url.into()), Entry {
            data: Some(open_pr_for_test(url)),
            fetched_at: Instant::now() - (CACHE_TTL + Duration::from_secs(60)),
        });
        let links = vec![mast_proto::ChatPrLink { url: url.into(), source: mast_proto::ChatLinkSource::CreatedInChat }];
        let status = cache.status_for_chat_links(None, None, None, &links, None);
        assert_eq!(status.pr_links[0].detail.as_ref().map(|pr| pr.state.as_str()), Some("OPEN"));
        assert!(lock(&cache.inner.pending_pr).contains(&PrKey::Url(url.into())), "stale entry must be refreshed");
        assert_eq!(cache.request_pr_url(url).and_then(|pr| pr).map(|pr| pr.state), Some("OPEN".into()));
    }

    fn age(cache: &PrTicketCache, key: &PrKey, by: Duration) {
        lock(&cache.inner.pr_cache).get_mut(key).unwrap().fetched_at = Instant::now() - by;
    }

    #[test]
    fn failed_lookup_keeps_the_entry_and_its_age_and_backs_off() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/7";
        let key = PrKey::Url(url.into());
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(open_pr_for_test(url)))]);
        age(&cache, &key, CACHE_TTL + Duration::from_secs(5));
        let before = lock(&cache.inner.pr_cache)[&key].fetched_at;
        let revision = cache.watch_pr_revision().borrow().revision;
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Error)]);
        assert_eq!(lock(&cache.inner.pr_cache)[&key].fetched_at, before, "a failure must not look like a fresh check");
        assert_eq!(cache.watch_pr_revision().borrow().revision, revision);
        assert!(cache.peek_pr_url(url).is_none(), "unverified data is not fresh");
        lock(&cache.inner.pending_pr).clear();
        cache.request_pr_url(url);
        assert!(lock(&cache.inner.pending_pr).is_empty(), "backing off after a transient error");
        lock(&cache.inner.pr_retry).clear();
        cache.request_pr_url(url);
        assert!(!lock(&cache.inner.pending_pr).is_empty());
    }

    #[test]
    fn not_found_clears_a_known_pr_and_reports_the_url() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/7";
        let key = PrKey::Url(url.into());
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(open_pr_for_test(url)))]);
        cache.publish_pr_results(vec![(key, PrLookup::NotFound)]);
        assert!(matches!(cache.peek_pr_url(url), Some(None)));
        assert_eq!(cache.watch_pr_revision().borrow().urls, vec![url.to_owned()]);
    }

    #[test]
    fn gh_not_found_output_is_told_apart_from_transient_errors() {
        assert!(gh_view_not_found("no pull requests found for branch \"x\""));
        assert!(gh_view_not_found("GraphQL: Could not resolve to a PullRequest with the number of 9."));
        assert!(!gh_view_not_found("error connecting to api.github.com"));
        assert!(!gh_view_not_found("HTTP 401: Bad credentials"));
    }

    #[test]
    fn a_no_change_publish_does_not_bump_the_revision() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/7";
        let key = PrKey::Url(url.into());
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(open_pr_for_test(url)))]);
        let first = cache.watch_pr_revision().borrow().clone();
        assert_eq!(first.urls, vec![url.to_owned()]);
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(open_pr_for_test(url)))]);
        assert_eq!(*cache.watch_pr_revision().borrow(), first);
        let mut merged = open_pr_for_test(url);
        merged.state = "MERGED".into();
        cache.publish_pr_results(vec![(key, PrLookup::Found(merged))]);
        assert_eq!(cache.watch_pr_revision().borrow().revision, first.revision + 1);
    }

    #[test]
    fn merged_entries_are_served_but_never_requeued() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/7";
        let key = PrKey::Url(url.into());
        let mut merged = open_pr_for_test(url);
        merged.state = "MERGED".into();
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(merged))]);
        age(&cache, &key, STALE_PR_LIMIT + Duration::from_secs(60));
        let links = vec![mast_proto::ChatPrLink { url: url.into(), source: mast_proto::ChatLinkSource::CreatedInChat }];
        let status = cache.status_for_chat_links(None, None, None, &links, None);
        assert_eq!(status.pr_links[0].detail.as_ref().map(|pr| pr.state.as_str()), Some("MERGED"));
        assert_eq!(cache.request_pr_url(url).and_then(|pr| pr).map(|pr| pr.state), Some("MERGED".into()));
        assert!(lock(&cache.inner.pending_pr).is_empty());
    }

    #[test]
    fn a_merged_branch_lookup_is_rechecked_but_a_merged_url_is_not() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/7";
        let mut merged = open_pr_for_test(url);
        merged.state = "MERGED".into();
        let key = PrKey::Branch { cwd: PathBuf::from("/tmp/repo-reuse"), branch: "reused".into() };
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(merged))]);
        age(&cache, &key, CACHE_TTL + Duration::from_secs(5));
        lock(&cache.inner.pending_pr).clear();
        cache.status_for_chat_links(None, Some("/tmp/repo-reuse"), Some("reused"), &[], None);
        assert!(lock(&cache.inner.pending_pr).contains(&key), "a reused branch can have a follow-up PR");
    }

    #[test]
    fn a_changed_branch_lookup_is_published_by_checkout_and_branch() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/9";
        let key = PrKey::Branch { cwd: PathBuf::from("/tmp/repo-a"), branch: "feature".into() };
        cache.publish_pr_results(vec![(key, PrLookup::Found(open_pr_for_test(url)))]);
        let update = cache.watch_pr_revision().borrow().clone();
        assert_eq!(update.branches, vec![PrBranchKey { cwd: "/tmp/repo-a".into(), branch: "feature".into() }]);
        assert_eq!(update.urls, vec![url.to_owned()]);
    }

    #[test]
    fn open_data_is_not_served_after_a_day_without_a_check() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/7";
        let key = PrKey::Url(url.into());
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(open_pr_for_test(url)))]);
        age(&cache, &key, STALE_PR_LIMIT + Duration::from_secs(60));
        let links = vec![mast_proto::ChatPrLink { url: url.into(), source: mast_proto::ChatLinkSource::CreatedInChat }];
        assert!(cache.status_for_chat_links(None, None, None, &links, None).pr_links[0].detail.is_none());
        assert!(cache.request_pr_url(url).is_none());
    }

    #[test]
    fn an_expired_inferred_pr_is_refreshed_even_with_durable_links() {
        let cache = PrTicketCache::new();
        let branch_pr = "https://github.com/example/repo/pull/8";
        let key = PrKey::Branch { cwd: PathBuf::from("/tmp/repo-stack"), branch: "stack".into() };
        cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(open_pr_for_test(branch_pr)))]);
        age(&cache, &key, CACHE_TTL + Duration::from_secs(5));
        let links = vec![mast_proto::ChatPrLink { url: "https://github.com/example/repo/pull/7".into(), source: mast_proto::ChatLinkSource::Manual }];
        lock(&cache.inner.pending_pr).clear();
        let status = cache.status_for_chat_links(None, Some("/tmp/repo-stack"), Some("stack"), &links, None);
        assert!(status.pr_links.iter().any(|link| link.url == branch_pr), "stale inferred PR is still shown");
        assert!(lock(&cache.inner.pending_pr).contains(&key));
    }

    #[test]
    fn pending_lookups_run_oldest_first() {
        let cache = PrTicketCache::new();
        let urls: Vec<String> = (1..=5).map(|n| format!("https://github.com/example/repo/pull/{n}")).collect();
        for (index, url) in urls.iter().enumerate() {
            let key = PrKey::Url(url.clone());
            cache.publish_pr_results(vec![(key.clone(), PrLookup::Found(open_pr_for_test(url)))]);
            age(&cache, &key, Duration::from_secs(1000 - index as u64 * 100));
            lock(&cache.inner.pending_pr).insert(key);
        }
        let take = cache.take_pending_prs();
        assert!(take == urls[..3].iter().map(|url| PrKey::Url(url.clone())).collect::<Vec<_>>());
    }

    #[test]
    fn a_no_pr_result_is_not_re_asked_every_cache_ttl() {
        let cache = PrTicketCache::new();
        let key = PrKey::Branch {
            cwd: PathBuf::from("/tmp/repo"),
            branch: "feature-without-pr".to_string(),
        };
        // A miss fetched 5 minutes ago: past CACHE_TTL, inside NEGATIVE_PR_TTL.
        lock(&cache.inner.pr_cache).insert(
            key.clone(),
            Entry {
                data: None,
                fetched_at: Instant::now() - (CACHE_TTL + Duration::from_secs(120)),
            },
        );
        cache.status_for(Some("/tmp/repo"), Some("feature-without-pr"), None, None);
        assert!(lock(&cache.inner.pending_pr).is_empty(), "miss re-queued too early");
        // Past the negative TTL it is asked again.
        lock(&cache.inner.pr_cache).insert(
            key,
            Entry {
                data: None,
                fetched_at: Instant::now() - (NEGATIVE_PR_TTL + Duration::from_secs(1)),
            },
        );
        cache.status_for(Some("/tmp/repo"), Some("feature-without-pr"), None, None);
        assert_eq!(lock(&cache.inner.pending_pr).len(), 1);
    }

    #[test]
    fn completion_observation_queue_is_bounded_and_rotates() {
        use crate::pr_merged_wakes::CompletionObservation;
        assert!(COMPLETION_SCAN_INTERVAL >= Duration::from_secs(60));
        let cache = PrTicketCache::new();
        let observations: Vec<_> = (0..20).map(|n| CompletionObservation::Url(format!("https://github.com/acme/repo/pull/{n}"))).collect();
        cache.queue_completion_observations(&cache.select_completion_observations(&observations));
        let first = lock(&cache.inner.pending_pr).clone();
        assert_eq!(first.len(), COMPLETION_SCAN_LIMIT);
        lock(&cache.inner.pending_pr).clear();
        cache.queue_completion_observations(&cache.select_completion_observations(&observations));
        let second = lock(&cache.inner.pending_pr).clone();
        assert_eq!(second.len(), COMPLETION_SCAN_LIMIT);
        assert_eq!(first.union(&second).count(), observations.len());
    }

    #[test]
    fn archive_cache_uses_stale_merged_but_not_stale_closed_prs() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/acme/repo/pull/1";
        let mut status = pr_json_to_status(&serde_json::json!({"number": 1, "state": "MERGED"}));
        lock(&cache.inner.pr_cache).insert(PrKey::Url(url.into()), Entry {
            data: Some(status.clone()), fetched_at: Instant::now() - CACHE_TTL - Duration::from_secs(1),
        });
        assert!(cache.cached_pr_state(url).is_some_and(|state| state.eq_ignore_ascii_case("merged")));
        status.state = "CLOSED".into();
        lock(&cache.inner.pr_cache).insert(PrKey::Url(url.into()), Entry {
            data: Some(status), fetched_at: Instant::now() - CACHE_TTL - Duration::from_secs(1),
        });
        assert_eq!(cache.cached_pr_state(url), None);
        assert!(lock(&cache.inner.pending_pr).is_empty());
    }

    #[test]
    fn status_for_registers_pending_on_miss_and_reads_cache_on_hit() {
        let cache = PrTicketCache::new();
        // First read: nothing cached yet — both sides None, key registered.
        let status = cache.status_for(Some("/tmp/repo"), Some("eng-2715-fix-thing"), None, None);
        assert!(status.pr.is_none());
        assert!(status.ticket.is_none());
        assert_eq!(lock(&cache.inner.pending_pr).len(), 1);
        assert_eq!(lock(&cache.inner.pending_ticket).len(), 1);

        // Manually seed the cache (as the sweep would) and confirm a fresh
        // read serves it without re-registering as pending.
        lock(&cache.inner.pending_pr).clear();
        lock(&cache.inner.pr_cache).insert(
            PrKey::Branch {
                cwd: PathBuf::from("/tmp/repo"),
                branch: "eng-2715-fix-thing".to_string(),
            },
            Entry {
                data: Some(PrStatus {
                    additions: None, deletions: None,
                    merged_at: None,
                    number: 7,
                    url: None,
                    state: "open".into(),
                    is_draft: false,
                    review_decision: None,
                    reviewers: Vec::new(),
                    checks: None,
                    title: None,
                    branch: None,
                    head_sha: None,
                    mergeable: "unknown".into(),
                    has_reviewer_requested: false,
                }),
                fetched_at: Instant::now(),
            },
        );
        let status = cache.status_for(Some("/tmp/repo"), Some("eng-2715-fix-thing"), None, None);
        assert_eq!(status.pr.map(|p| p.number), Some(7));
        assert!(lock(&cache.inner.pending_pr).is_empty());
    }

    #[test]
    fn selected_checkout_is_the_only_branch_pr_lookup() {
        let cache = PrTicketCache::new();
        cache.status_for_chat_links_with_checkout(
            None,
            Some("/tmp/root"),
            Some("root-branch"),
            &[],
            None,
            Some(("/tmp/root/submodule", Some("checkout-branch"))),
        );
        let pending = lock(&cache.inner.pending_pr);
        assert_eq!(pending.len(), 1);
        assert!(pending.contains(&PrKey::CheckoutBranch {
            cwd: PathBuf::from("/tmp/root/submodule"),
            branch: "checkout-branch".into()
        }));
    }

    #[test]
    fn detached_checkout_does_not_enqueue_a_branch_lookup() {
        let cache = PrTicketCache::new();
        cache.status_for_chat_links_with_checkout(
            None,
            Some("/tmp/root"),
            Some("root-branch"),
            &[],
            None,
            Some(("/tmp/target", None)),
        );
        assert!(lock(&cache.inner.pending_pr).is_empty());
    }

    #[test]
    fn durable_pr_link_survives_a_selected_checkout() {
        let cache = PrTicketCache::new();
        let links = vec![mast_proto::ChatPrLink {
            url: "https://github.com/acme/app/pull/3".into(),
            source: mast_proto::ChatLinkSource::Manual,
        }];
        let status = cache.status_for_chat_links_with_checkout(
            None,
            Some("/tmp/root"),
            Some("root"),
            &links,
            None,
            Some(("/tmp/target", Some("feature"))),
        );
        assert_eq!(status.pr_links.len(), 1);
        assert_eq!(status.pr_links[0].url, links[0].url);
    }

    #[test]
    fn peek_pr_url_does_not_queue_work_and_distinguishes_a_cached_miss() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/1";
        assert!(cache.peek_pr_url(url).is_none());
        assert!(lock(&cache.inner.pending_pr).is_empty());
        lock(&cache.inner.pr_cache).insert(PrKey::Url(url.into()), Entry {
            data: None,
            fetched_at: Instant::now(),
        });
        assert!(matches!(cache.peek_pr_url(url), Some(None)));
        assert!(lock(&cache.inner.pending_pr).is_empty());
    }

    #[test]
    fn peek_pr_branch_uses_the_correct_cache_namespace_without_queuing() {
        let cache = PrTicketCache::new();
        let key = PrKey::CheckoutBranch { cwd: PathBuf::from("/repo"), branch: "feature".into() };
        lock(&cache.inner.pr_cache).insert(key, Entry { data: None, fetched_at: Instant::now() });
        assert!(matches!(cache.peek_pr_branch("/repo", "feature", true), Some(None)));
        assert!(cache.peek_pr_branch("/repo", "feature", false).is_none());
        assert!(lock(&cache.inner.pending_pr).is_empty());
    }

    #[test]
    fn requesting_an_item_pr_only_queues_valid_uncached_urls() {
        let cache = PrTicketCache::new();
        let url = "https://github.com/example/repo/pull/1";
        assert!(cache.request_pr_url("not a PR").is_none());
        assert!(lock(&cache.inner.pending_pr).is_empty());
        assert!(cache.request_pr_url(url).is_none());
        assert_eq!(lock(&cache.inner.pending_pr).len(), 1);
        lock(&cache.inner.pending_pr).clear();
        assert!(cache.request_pr_url("--unexpected=github.com/example/repo/pull/1").is_none());
        assert!(lock(&cache.inner.pending_pr).contains(&PrKey::Url(url.into())));
        lock(&cache.inner.pending_pr).clear();
        lock(&cache.inner.pr_cache).insert(PrKey::Url(url.into()), Entry { data: None, fetched_at: Instant::now() });
        assert!(matches!(cache.request_pr_url(url), Some(None)));
        assert!(lock(&cache.inner.pending_pr).is_empty());
    }

    #[tokio::test]
    async fn pr_cache_watch_emits_after_a_batch_that_changed_something() {
        let cache = PrTicketCache::new();
        let mut revisions = cache.watch_pr_revision();
        let url = "https://github.com/example/repo/pull/1";
        cache.publish_pr_results(vec![(PrKey::Url(url.into()), PrLookup::Found(open_pr_for_test(url)))]);
        revisions.changed().await.unwrap();
        assert_eq!(revisions.borrow_and_update().revision, 1);
        cache.publish_pr_results(Vec::new());
        assert!(!revisions.has_changed().unwrap());
    }

    #[test]
    fn status_for_links_keeps_every_durable_pr_while_details_load() {
        use mast_proto::{ChatLinkSource, ChatPrLink};

        let cache = PrTicketCache::new();
        let links = vec![
            ChatPrLink {
                url: "https://github.com/acme/widgets/pull/7".into(),
                source: ChatLinkSource::Manual,
            },
            ChatPrLink {
                url: "https://github.com/acme/widgets/pull/8".into(),
                source: ChatLinkSource::CreatedInChat,
            },
        ];

        let status = cache.status_for_links(None, None, &links, None);
        assert_eq!(status.pr_links.len(), 2);
        assert_eq!(status.pr_links[0].url, links[0].url);
        assert_eq!(status.pr_links[0].source, Some(ChatLinkSource::Manual));
        assert!(status.pr_links.iter().all(|link| link.detail.is_none()));
        assert_eq!(lock(&cache.inner.pending_pr).len(), 2);

        lock(&cache.inner.pr_cache).insert(
            PrKey::Url(links[1].url.clone()),
            Entry {
                data: Some(PrStatus {
                    additions: None, deletions: None,
                    merged_at: None,
                    number: 8,
                    url: Some(links[1].url.clone()),
                    state: "open".into(),
                    is_draft: false,
                    review_decision: None,
                    reviewers: Vec::new(),
                    checks: None,
                    title: Some("Second PR".into()),
                    branch: None,
                    head_sha: None,
                    mergeable: "unknown".into(),
                    has_reviewer_requested: true,
                }),
                fetched_at: Instant::now(),
            },
        );
        let status = cache.status_for_links(None, None, &links, None);
        assert_eq!(
            status.pr_links[1].detail.as_ref().map(|pr| pr.number),
            Some(8)
        );
        assert_eq!(status.pr_links[0].url, links[0].url);
    }

    #[test]
    fn status_for_with_no_branch_is_a_pure_noop() {
        let cache = PrTicketCache::new();
        let status = cache.status_for(Some("/tmp/repo"), None, None, None);
        assert!(status.pr.is_none());
        assert!(status.ticket.is_none());
        assert!(lock(&cache.inner.pending_pr).is_empty());
        assert!(lock(&cache.inner.pending_ticket).is_empty());
    }

    /// Ticket 0xx lookup-order change: a chat with a durable `linked_pr_url`
    /// resolves PR detail by that URL, NOT the branch guess — even when a
    /// cwd+branch are also given (the two are never combined into the same
    /// key).
    #[test]
    fn status_for_prefers_linked_pr_url_over_the_branch_guess() {
        let cache = PrTicketCache::new();
        let status = cache.status_for(
            Some("/tmp/repo"),
            Some("eng-2715-fix-thing"),
            Some("https://github.com/acme/widgets/pull/42"),
            None,
        );
        assert!(status.pr.is_none(), "nothing cached yet");
        let pending = lock(&cache.inner.pending_pr);
        assert_eq!(pending.len(), 1);
        assert!(pending.contains(&PrKey::Url(
            "https://github.com/acme/widgets/pull/42".to_string()
        )));
        assert!(!pending.contains(&PrKey::Branch {
            cwd: PathBuf::from("/tmp/repo"),
            branch: "eng-2715-fix-thing".to_string(),
        }));
    }

    #[test]
    fn status_for_links_does_not_queue_branch_guess_for_linked_prs() {
        use mast_proto::{ChatLinkSource, ChatPrLink};

        let cache = PrTicketCache::new();
        let links = vec![
            ChatPrLink {
                url: "https://github.com/acme/widgets/pull/42".into(),
                source: ChatLinkSource::Manual,
            },
            ChatPrLink {
                url: "https://github.com/acme/widgets/pull/43".into(),
                source: ChatLinkSource::Mentioned,
            },
        ];

        cache.status_for_links(Some("/tmp/repo"), Some("eng-2715-fix-thing"), &links, None);

        let pending = lock(&cache.inner.pending_pr);
        assert_eq!(pending.len(), 2);
        assert!(
            links
                .iter()
                .all(|link| pending.contains(&PrKey::Url(link.url.clone())))
        );
        assert!(!pending.contains(&PrKey::Branch {
            cwd: PathBuf::from("/tmp/repo"),
            branch: "eng-2715-fix-thing".to_string(),
        }));
    }

    /// Sibling to the above: a link-LESS chat keeps exactly the pre-existing
    /// branch-based `Branch` key.
    #[test]
    fn status_for_falls_back_to_the_branch_guess_when_link_less() {
        let cache = PrTicketCache::new();
        let status = cache.status_for(Some("/tmp/repo"), Some("eng-2715-fix-thing"), None, None);
        assert!(status.pr.is_none());
        let pending = lock(&cache.inner.pending_pr);
        assert!(pending.contains(&PrKey::Branch {
            cwd: PathBuf::from("/tmp/repo"),
            branch: "eng-2715-fix-thing".to_string(),
        }));
    }

    #[test]
    fn changing_one_linear_account_invalidates_only_its_tickets() {
        let cache = PrTicketCache::new();
        let default = TicketKey::Linear { identifier: "ENG-1".into(), account_id: None, credential_revision: 0 };
        let personal = TicketKey::Linear { identifier: "ENG-1".into(), account_id: Some("personal".into()), credential_revision: 0 };
        for key in [default.clone(), personal.clone()] {
            lock(&cache.inner.ticket_cache).insert(key, Entry { data: None, fetched_at: Instant::now() });
        }
        cache.invalidate_linear_account("personal");
        assert!(lock(&cache.inner.ticket_cache).contains_key(&default));
        assert!(!lock(&cache.inner.ticket_cache).contains_key(&personal));
        assert_eq!(lock(&cache.inner.linear_ticket_revisions).get(&Some("personal".into())), Some(&1));
    }

    #[test]
    fn ticket_cache_keys_include_linear_account() {
        let cache = PrTicketCache::new();
        for account in [None, Some("personal".to_owned())] {
            cache.status_for_links_with_checkout(None, None, &[], Some("ENG-1"), None, Some((account, 0)));
        }
        let pending = lock(&cache.inner.pending_ticket);
        assert!(pending.contains(&TicketKey::Linear { identifier: "ENG-1".into(), account_id: None, credential_revision: 0 }));
        assert!(pending.contains(&TicketKey::Linear { identifier: "ENG-1".into(), account_id: Some("personal".into()), credential_revision: 0 }));
        assert_eq!(pending.len(), 2);
        drop(pending);
        cache.status_for_links_with_checkout(None, None, &[], Some("ENG-1"), None, Some((None, 1)));
        assert_eq!(lock(&cache.inner.pending_ticket).len(), 3);
    }

    #[test]
    fn status_for_never_queues_cwd_probes_for_privacy_managed_history() {
        let home = crate::repos::home_dir();
        for child in ["Music", "Pictures", "Documents"] {
            let cache = PrTicketCache::new();
            let cwd = home
                .join(child)
                .join(".agent-worktrees")
                .join("historical-chat");
            let status = cache.status_for(cwd.to_str(), Some("eng-2715-fix-thing"), None, None);

            assert!(status.pr.is_none());
            assert!(!status.is_worktree);
            assert!(lock(&cache.inner.pending_pr).is_empty());
            assert!(lock(&cache.inner.pending_diff_stat).is_empty());
            assert!(
                lock(&cache.inner.pending_ticket).contains(&TicketKey::Cli("ENG-2715".into())),
                "ticket inference is path-independent and should remain available"
            );
        }
    }

    #[test]
    fn status_for_still_queues_ordinary_project_cwds() {
        let cache = PrTicketCache::new();
        let cwd = crate::repos::home_dir()
            .join("Projects")
            .join("zeron")
            .join(".agent-worktrees")
            .join("feature");
        cache.status_for(cwd.to_str(), Some("feature/privacy"), None, None);

        assert!(lock(&cache.inner.pending_pr).contains(&PrKey::Branch {
            cwd: cwd.clone(),
            branch: "feature/privacy".into(),
        }));
        assert!(lock(&cache.inner.pending_diff_stat).contains(&cwd));
    }

    /// Same lookup-order change, ticket side: a durable `linked_ticket_id`
    /// wins over the branch-name regex guess.
    #[test]
    fn status_for_prefers_linked_ticket_id_over_the_branch_regex() {
        let cache = PrTicketCache::new();
        let status = cache.status_for(
            Some("/tmp/repo"),
            Some("eng-2715-fix-thing"),
            None,
            Some("OPS-9"),
        );
        assert!(status.ticket.is_none(), "nothing cached yet");
        let pending = lock(&cache.inner.pending_ticket);
        assert!(pending.contains(&TicketKey::Cli("OPS-9".into())));
        assert!(!pending.contains(&TicketKey::Cli("ENG-2715".into())));
    }

    #[test]
    fn is_worktree_cwd_matches_both_known_layouts() {
        assert!(is_worktree_cwd(
            "/Users/example/Projects/cofactr/workspace/.worktrees/eng-1"
        ));
        assert!(is_worktree_cwd(
            "/Users/example/Projects/cofactr/workspace/.agent-worktrees/ENG-2713"
        ));
        assert!(!is_worktree_cwd("/Users/example/Projects/cofactr/workspace"));
        // A substring match elsewhere in the path must not false-positive —
        // the check is specifically for the `/.worktrees/`/`.agent-worktrees/`
        // path segment, not any occurrence of the word.
        assert!(!is_worktree_cwd(
            "/Users/example/Projects/worktrees-are-fun/workspace"
        ));
    }

    #[test]
    fn parse_git_shortstat_handles_every_clause_shape() {
        assert_eq!(
            parse_git_shortstat(" 3 files changed, 10 insertions(+), 4 deletions(-)"),
            DiffStat {
                files_changed: 3,
                lines_added: 10,
                lines_removed: 4,
            }
        );
        // Pure addition: no "deletions" clause at all.
        assert_eq!(
            parse_git_shortstat(" 1 file changed, 1 insertion(+)"),
            DiffStat {
                files_changed: 1,
                lines_added: 1,
                lines_removed: 0,
            }
        );
        // Pure deletion: no "insertions" clause at all.
        assert_eq!(
            parse_git_shortstat(" 1 file changed, 3 deletions(-)"),
            DiffStat {
                files_changed: 1,
                lines_added: 0,
                lines_removed: 3,
            }
        );
        // A clean tree: `git diff --shortstat` prints nothing.
        assert_eq!(parse_git_shortstat(""), DiffStat::default());
        assert_eq!(parse_git_shortstat("\n"), DiffStat::default());
    }

    #[test]
    fn status_for_computes_is_worktree_without_touching_the_diff_stat_cache_for_non_worktrees() {
        let cache = PrTicketCache::new();
        let status = cache.status_for(
            Some("/Users/example/Projects/cofactr/workspace"),
            None,
            None,
            None,
        );
        assert!(!status.is_worktree);
        assert!(status.diff_stat.is_none());
        assert!(
            lock(&cache.inner.pending_diff_stat).is_empty(),
            "a non-worktree cwd must never register for a diff-stat sweep"
        );
    }

    #[test]
    fn status_for_registers_a_worktree_cwd_as_pending_then_reads_the_cache_on_hit() {
        let cache = PrTicketCache::new();
        let cwd = "/Users/example/Projects/cofactr/workspace/.agent-worktrees/ENG-2713";
        let status = cache.status_for(Some(cwd), None, None, None);
        assert!(status.is_worktree);
        assert!(status.diff_stat.is_none(), "nothing cached yet");
        assert_eq!(lock(&cache.inner.pending_diff_stat).len(), 1);

        lock(&cache.inner.pending_diff_stat).clear();
        lock(&cache.inner.diff_stat_cache).insert(
            PathBuf::from(cwd),
            Entry {
                data: Some(DiffStat {
                    files_changed: 2,
                    lines_added: 5,
                    lines_removed: 1,
                }),
                fetched_at: Instant::now(),
            },
        );
        let status = cache.status_for(Some(cwd), None, None, None);
        assert_eq!(
            status.diff_stat,
            Some(DiffStat {
                files_changed: 2,
                lines_added: 5,
                lines_removed: 1,
            })
        );
        assert!(lock(&cache.inner.pending_diff_stat).is_empty());
    }

    /// Wire-contract pin: the overview UI (`crates/ui/src/overview.rs`,
    /// outside this crate) deserializes `ChatLinkStatus`'s raw JSON directly
    /// and depends on these EXACT camelCase field names —
    /// `isWorktree`/`diffStat.{filesChanged,linesAdded,linesRemoved}`/
    /// `prSource`/`ticketSource`. A silent rename here (e.g. from a
    /// `#[serde(rename_all)]` refactor) would compile fine on this side and
    /// just silently stop populating the UI's tiles, so this pins the exact
    /// JSON shape rather than only the Rust struct fields the tests above
    /// already cover.
    #[test]
    fn chat_link_status_json_field_names_match_the_overview_ui_contract() {
        let status = ChatLinkStatus {
            pr: None,
            pr_links: Vec::new(),
            ticket: None,
            is_worktree: true,
            diff_stat: Some(DiffStat {
                files_changed: 2,
                lines_added: 5,
                lines_removed: 1,
            }),
            pr_source: Some(mast_proto::ChatLinkSource::Manual),
            ticket_source: None,
        };
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["isWorktree"], serde_json::json!(true));
        assert_eq!(
            value["diffStat"],
            serde_json::json!({"filesChanged": 2, "linesAdded": 5, "linesRemoved": 1})
        );
        // `prSource` round-trips the snake_case wire spelling the design
        // fixes (`"manual" | "created_in_chat" | "mentioned"`), even though
        // every OTHER field on this struct is camelCase.
        assert_eq!(value["prSource"], serde_json::json!("manual"));
        assert_eq!(value["ticketSource"], serde_json::Value::Null);

        // Non-worktree shape: `isWorktree: false`, `diffStat: null`,
        // `prSource`/`ticketSource: null` — the UI is told explicitly, not
        // left to infer absence-means-false/inferred.
        let non_worktree = ChatLinkStatus::default();
        let value = serde_json::to_value(&non_worktree).unwrap();
        assert_eq!(value["isWorktree"], serde_json::json!(false));
        assert_eq!(value["diffStat"], serde_json::Value::Null);
        assert_eq!(value["prSource"], serde_json::Value::Null);
        assert_eq!(value["ticketSource"], serde_json::Value::Null);
    }

    /// Live smoke test: this very repo's own checkout, not a worktree, so
    /// `git diff --shortstat HEAD` should run cleanly and return *some*
    /// parseable stat (zero or not, depending on the working tree at test
    /// time) — proves the subprocess plumbing, not specific content.
    /// `#[ignore]`: depends on `git` being on PATH.
    #[test]
    #[ignore = "depends on `git` being installed"]
    fn live_git_diff_shortstat_against_this_repo_runs_and_parses() {
        let cwd = std::env::current_dir().unwrap();
        assert!(git_diff_shortstat(&cwd).is_some());
    }

    /// Live smoke test against this very repo's own remote — `gh` is
    /// installed on this machine (confirmed: `linear` is not, so no
    /// equivalent ticket-side live test is possible here). Real evidence the
    /// subprocess plumbing actually works end to end, not just against fake
    /// JSON fixtures. `#[ignore]`: depends on network + `gh` auth state on
    /// whatever machine runs the suite.
    #[test]
    #[ignore = "depends on `gh` being installed and authenticated, and on network access"]
    fn live_gh_pr_lookup_against_a_real_repo() {
        // main/master branch of the zeronsh/zeron upstream is very unlikely
        // to have an open PR against it, so this only proves plumbing
        // (spawns gh, gets a clean non-error `None`), not the full parse
        // path — that's covered by the fixture-based tests above.
        let cwd = std::env::current_dir().unwrap();
        let result = fetch_github_pr(&cwd, "this-branch-should-not-exist-anywhere");
        assert!(!matches!(result, PrLookup::Found(_)), "a bogus branch must not fabricate a PR");
    }

    fn sample_my_pr(number: u64, repo: &str) -> MyOpenPr {
        MyOpenPr {
            number,
            title: Some(format!("PR #{number}")),
            url: Some(format!("https://github.com/{repo}/pull/{number}")),
            repo: Some(repo.to_string()),
            is_draft: false,
            updated_at: None,
            author: None,
        }
    }

    #[test]
    fn my_open_prs_is_empty_before_any_sweep_has_populated_the_list() {
        let cache = PrTicketCache::new();
        assert!(cache.my_open_prs().is_empty());
    }

    #[test]
    fn my_open_prs_pairs_list_entries_with_their_cached_detail() {
        let cache = PrTicketCache::new();
        *lock(&cache.inner.my_prs_list) = Some((
            vec![
                sample_my_pr(1, "acme/widgets"),
                sample_my_pr(2, "acme/widgets"),
            ],
            Instant::now(),
        ));
        // Only #1 has detail cached — #2 must read back with `detail: None`,
        // not panic or get skipped.
        lock(&cache.inner.my_pr_detail).insert(
            ("acme/widgets".to_string(), 1),
            Entry {
                data: Some(PrStatus {
                    additions: None, deletions: None,
                    merged_at: None,
                    number: 1,
                    url: Some("https://github.com/acme/widgets/pull/1".into()),
                    state: "open".into(),
                    is_draft: false,
                    review_decision: None,
                    reviewers: Vec::new(),
                    checks: Some(ChecksStatus::Failing),
                    title: None,
                    branch: None,
                    head_sha: None,
                    mergeable: "unknown".into(),
                    // `true` here isolates this test's actual point (detail
                    // pairing, not action-classification breadth) — `false`
                    // would also add `NeedsReviewer`, already covered by
                    // `action_reasons_matches_reference_classification`.
                    has_reviewer_requested: true,
                }),
                fetched_at: Instant::now(),
            },
        );

        let items = cache.my_open_prs();
        assert_eq!(items.len(), 2);
        let pr1 = items.iter().find(|i| i.summary.number == 1).unwrap();
        assert!(pr1.detail.is_some());
        assert_eq!(
            pr1.detail.as_ref().unwrap().action_reasons(),
            vec![PrAction::CiFailing]
        );
        let pr2 = items.iter().find(|i| i.summary.number == 2).unwrap();
        assert!(
            pr2.detail.is_none(),
            "PR with no cached detail yet must read back as None, not panic"
        );
    }

    #[test]
    fn my_prs_list_staleness_is_ttl_gated() {
        // Fresh entry: not stale.
        let fresh = (vec![sample_my_pr(1, "a/b")], Instant::now());
        assert!(fresh.1.elapsed() < MY_PRS_LIST_TTL);

        // An entry old enough must be picked up as needing a re-sweep — the
        // actual sweep is exercised live in `sweep_my_prs_list`, but the TTL
        // comparison itself (what `sweep_my_prs_list` guards on) is what a
        // unit test can pin down without shelling out.
        let stale_at = Instant::now()
            .checked_sub(MY_PRS_LIST_TTL + Duration::from_secs(1))
            .expect("MY_PRS_LIST_TTL is small enough that now() - it - 1s doesn't underflow");
        assert!(stale_at.elapsed() >= MY_PRS_LIST_TTL);
    }

    #[test]
    fn review_requested_prs_is_empty_before_sweep_and_has_no_detail() {
        let cache = PrTicketCache::new();
        assert!(cache.review_requested_prs().is_empty());
        let mut pr = sample_my_pr(7, "acme/widgets");
        pr.author = Some("octocat".into());
        *lock(&cache.inner.review_prs_list) = Some((vec![pr], Instant::now()));
        // Even with detail cached for the same key, review rows skip detail.
        let items = cache.review_requested_prs();
        assert_eq!(items.len(), 1);
        assert!(items[0].detail.is_none());
        assert_eq!(items[0].summary.author.as_deref(), Some("octocat"));
        // Independent slot: authored list untouched.
        assert!(cache.my_open_prs().is_empty());
    }

    #[test]
    fn parse_pr_search_reads_author_login_from_review_requested_fixture() {
        let fixture = br#"[
            {"number": 12, "title": "Fix it", "url": "https://github.com/acme/widgets/pull/12",
             "repository": {"name": "widgets", "nameWithOwner": "acme/widgets"},
             "isDraft": false, "updatedAt": "2026-09-01T00:00:00Z",
             "author": {"login": "octocat", "is_bot": false, "type": "User", "url": "https://github.com/octocat"}},
            {"number": 13, "repository": {"nameWithOwner": "acme/widgets"}, "isDraft": true}
        ]"#;
        let items = parse_pr_search(fixture).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].author.as_deref(), Some("octocat"));
        assert_eq!(items[0].repo.as_deref(), Some("acme/widgets"));
        assert!(items[1].is_draft);
        assert_eq!(items[1].author, None);
        // Authored-list fixture (no `author` key) still parses.
        assert_eq!(parse_pr_search(b"[]").unwrap().len(), 0);
        assert!(parse_pr_search(b"not json").is_none());
    }

    /// Pins the wire shape the overview UI reads: camelCase keys, `author`
    /// present only when set, and flattened into `MyPrItem`.
    #[test]
    fn my_pr_item_json_field_names_match_the_overview_ui_contract() {
        let mut pr = sample_my_pr(3, "acme/widgets");
        pr.is_draft = true;
        pr.updated_at = Some("2026-09-01T00:00:00Z".into());
        let authored = serde_json::to_value(MyPrItem {
            summary: pr.clone(),
            detail: None,
        })
        .unwrap();
        assert_eq!(authored["number"], serde_json::json!(3));
        assert_eq!(authored["repo"], serde_json::json!("acme/widgets"));
        assert_eq!(authored["isDraft"], serde_json::json!(true));
        assert_eq!(
            authored["updatedAt"],
            serde_json::json!("2026-09-01T00:00:00Z")
        );
        assert_eq!(authored["detail"], serde_json::Value::Null);
        assert!(
            authored.get("author").is_none(),
            "authored payload must not grow an author key"
        );

        pr.author = Some("octocat".into());
        let review = serde_json::to_value(MyPrItem {
            summary: pr,
            detail: None,
        })
        .unwrap();
        assert_eq!(review["author"], serde_json::json!("octocat"));
    }

    #[test]
    fn search_due_honors_ttl_and_failure_backoff() {
        let now = Instant::now();
        let ago = |secs| now.checked_sub(Duration::from_secs(secs)).unwrap();
        // Never fetched, never failed: due.
        assert!(search_due(None, None));
        // Fresh list: not due. Stale list: due.
        assert!(!search_due(Some(ago(5)), None));
        assert!(search_due(Some(ago(61)), None));
        // Failed 20s ago: backed off even though there is no list.
        assert!(!search_due(None, Some(ago(20))));
        // Backoff window (90s) elapsed: retry.
        assert!(search_due(None, Some(ago(91))));
        // Backoff also suppresses a stale list's refresh.
        assert!(!search_due(Some(ago(300)), Some(ago(30))));
    }

    #[tokio::test]
    async fn failed_search_records_failure_and_does_not_retry_within_backoff() {
        fn fail() -> Option<Vec<MyOpenPr>> {
            None
        }
        fn ok() -> Option<Vec<MyOpenPr>> {
            Some(vec![])
        }
        let list = Mutex::new(None);
        let failed = Mutex::new(None);
        assert!(
            sweep_search_slot(&list, &failed, fail).await,
            "first tick attempts"
        );
        assert!(failed.lock().unwrap().is_some());
        assert!(list.lock().unwrap().is_none());
        // Next tick (20s later in prod) is inside the 90s backoff: no attempt,
        // even with a fetcher that would succeed.
        assert!(!sweep_search_slot(&list, &failed, ok).await);
        assert!(list.lock().unwrap().is_none());
        // Backoff elapsed: retries and clears the failure marker.
        *failed.lock().unwrap() =
            Instant::now().checked_sub(SEARCH_FAILURE_BACKOFF + Duration::from_secs(1));
        assert!(sweep_search_slot(&list, &failed, ok).await);
        assert!(failed.lock().unwrap().is_none());
        assert!(list.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn authored_pr_readiness_waits_for_first_success_or_failure() {
        fn succeed() -> Option<Vec<MyOpenPr>> {
            Some(Vec::new())
        }
        fn fail() -> Option<Vec<MyOpenPr>> {
            None
        }

        let successful = PrTicketCache::new();
        assert!(
            !successful
                .wait_for_my_open_prs_ready(Duration::from_millis(1))
                .await
        );
        assert!(successful.sweep_my_prs_list_with(succeed).await);
        assert!(
            successful
                .wait_for_my_open_prs_ready(Duration::from_millis(1))
                .await
        );

        let failed = PrTicketCache::new();
        assert!(failed.sweep_my_prs_list_with(fail).await);
        assert!(lock(&failed.inner.my_prs_failed_at).is_some());
        assert!(
            failed
                .wait_for_my_open_prs_ready(Duration::from_millis(1))
                .await
        );
    }

    /// Live smoke test: `gh search prs --author=@me --state=open` (no
    /// `--owner`) actually runs and returns parseable JSON on this machine.
    /// Doesn't assert on *content* (an empty result is a perfectly valid
    /// "no open PRs right now") — only that the subprocess+parse path works
    /// end to end, same evidentiary bar as `live_gh_pr_lookup_against_a_real_repo`.
    #[test]
    #[ignore = "depends on `gh` being installed and authenticated, and on network access"]
    fn live_my_open_prs_search_runs_and_parses() {
        let result = fetch_my_open_prs();
        assert!(
            result.is_some(),
            "gh search prs must return parseable JSON, even if empty"
        );
    }
}
