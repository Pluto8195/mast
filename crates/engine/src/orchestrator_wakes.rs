//! Worker-event wakes for orchestrator chats.
//!
//! An orchestrator is a chat whose workers are its child chats (`create_chat`
//! sets `parentChatId`) or chats on items under its project's worker
//! destination. The engine watches what those workers do, from the events it
//! already has: the Work Outline snapshot (Required Actions raised/resolved),
//! the session watch (errored, turn finished) and chat rows (PR linked). There
//! is no polling loop; the only timers are one-shot sleeps to the end of a
//! hold window.
//!
//! Events are batched per orchestrator ([`core::WAKE_WINDOW_MS`]) and delivered
//! as ONE system-attributed message, only while the orchestrator is idle and
//! the user is not in the chat. The user always goes first: a user message
//! pulls back a wake that has not started and interrupts an event-started
//! turn (see `DocHost::yield_event_turn_to_user`).

mod auto_continue;
mod core;
mod store;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use futures::FutureExt;

use mast_proto::{
    Chat, Session, SessionStatus, WakeEventKind, WakeSettings, WakeStatus, WorkOutlineActorKind,
    WorkOutlineSnapshot,
};

pub use self::core::{
    Batch, Ctx, DEDUPE_TTL_MS, Decision, USER_QUIET_MS, WAKE_WINDOW_MS, WakeCore, WakeEvent,
    resolved_key,
};
use crate::doc_host::DocHost;
use crate::sessions::SessionsEngine;
use crate::workspace_host::WorkspaceHost;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Epoch milliseconds.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| chrono::Utc::now().timestamp_millis())
}

fn materialize_due_outline<T>(
    space_id: &str,
    inputs: &mast_doc::WorkOutlineInputs,
    now: i64,
    materialize: impl FnOnce() -> T,
) -> Option<T> {
    let due = inputs.items.iter().any(|row| {
        row.fields.get("spaceId").and_then(|v| v.as_str()) == Some(space_id)
            && row.fields.get("waitingUntil").and_then(|v| v.as_i64()).is_some_and(|until| until <= now)
            && row.fields.get("archivedAt").and_then(|v| v.as_i64()).is_none()
            && row.fields.get("status").and_then(|v| v.as_str()) != Some("completed")
    });
    due.then(materialize)
}

#[cfg(test)]
mod waiting_outline_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn no_due_items_skip_materialization() {
        let mut inputs = mast_doc::WorkOutlineInputs::default();
        let mut row: mast_doc::RegistryRow = serde_json::from_value(serde_json::json!({
            "kind": "workItems", "id": "item", "fields": {
                "spaceId": "space", "waitingUntil": 101
            }
        })).unwrap();
        inputs.items.push(row.clone());
        let calls = Cell::new(0);
        let materialize = || { calls.set(calls.get() + 1); };
        assert_eq!(materialize_due_outline("space", &inputs, 100, materialize), None);
        assert_eq!(calls.get(), 0);
        row.fields.insert("waitingUntil".into(), serde_json::json!(100));
        inputs.items[0] = row;
        assert_eq!(materialize_due_outline("space", &inputs, 100, materialize), Some(()));
        assert_eq!(calls.get(), 1);
    }
}

// The original file was a flat map of chat ids to settings. Preserve it on
// read; missing autoContinue fields inherit the global default.
fn read_settings(bytes: &[u8]) -> (bool, HashMap<String, WakeSettings>) {
    let Ok(serde_json::Value::Object(mut root)) = serde_json::from_slice(bytes) else {
        return (false, HashMap::new());
    };
    let default = root
        .remove("autoContinue")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let entries = match root.remove("orchestrators") {
        Some(serde_json::Value::Object(entries)) => entries,
        _ => root,
    };
    let overrides = entries
        .into_iter()
        .filter_map(|(chat, mut value)| {
            if let Some(object) = value.as_object_mut() {
                object
                    .entry("autoContinue")
                    .or_insert(serde_json::Value::Bool(default));
            }
            serde_json::from_value(value)
                .ok()
                .map(|settings| (chat, settings))
        })
        .collect();
    (default, overrides)
}

#[derive(Debug)]
struct EventTurn {
    message_id: String,
    events: Vec<WakeEvent>,
    /// The drain took the queue row.
    dispatched: bool,
    /// The turn was seen in flight.
    started: bool,
}

/// Wake messages in flight, shared between the wake service and the doc host
/// (which makes way for the user).
#[derive(Debug, Default)]
pub struct EventTurns {
    turns: Mutex<HashMap<String, EventTurn>>,
    cancelled: Mutex<HashMap<String, Vec<WakeEvent>>>,
    persistence: Mutex<Option<WakePersistence>>,
}

#[derive(Clone)]
struct WakePersistence {
    path: PathBuf,
    store: store::WakeStore,
    generations: HashMap<(String, String), (Arc<crate::chat_persistence::ChatPersistence>, u64)>,
    retired_generations: HashMap<(String, String), (Arc<crate::chat_persistence::ChatPersistence>, u64)>,
    legacy_receipts: HashMap<(String, String), std::time::Instant>,
    legacy_retired: HashMap<(String, String), std::time::Instant>,
    unavailable: HashSet<String>,
}

impl std::fmt::Debug for WakePersistence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("WakePersistence").field("path", &self.path).finish_non_exhaustive()
    }
}

impl WakePersistence {
    fn prune(&mut self) {
        let mut retired: Vec<_> = self.retired_generations.iter()
            .filter(|(_, (snapshot, generation))| snapshot.has_saved(*generation))
            .map(|(key, _)| key.clone()).collect();
        retired.extend(self.legacy_retired.iter()
            .filter(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(3))
            .map(|(key, _)| key.clone()));
        for (chat, message) in retired {
            let mut referenced = false;
            for entries in self.store.orchestrators.values_mut() {
                for entry in entries {
                    if entry.message_id.as_deref() == Some(&message) { referenced = true; }
                    entry.retired_messages.retain(|retired| retired.chat_id != chat || retired.message_id != message);
                }
            }
            if !referenced {
                self.retired_generations.remove(&(chat.clone(), message.clone()));
                self.legacy_retired.remove(&(chat, message));
            }
        }
        for entries in self.store.orchestrators.values_mut() {
            entries.retain(|entry| !entry.discarded || entry.message_id.is_some() || !entry.retired_messages.is_empty());
        }
        let saved: Vec<_> = self.generations.iter()
            .filter(|(_, (snapshot, generation))| snapshot.has_saved(*generation))
            .map(|(key, _)| key.clone()).collect();
        for (chat, message) in saved {
            let mut completed = Vec::new();
            if let Some(entries) = self.store.orchestrators.get_mut(&chat) {
                entries.retain(|entry| {
                    let prune = entry.delivered && entry.retired_messages.is_empty() && entry.message_id.as_deref() == Some(&message);
                    if prune { completed.push(entry.event.id.clone()); }
                    !prune
                });
            }
            for id in completed { self.store.remember_completed(&chat, &id); }
            let retained = self.store.orchestrators.get(&chat).is_some_and(|entries| entries.iter().any(|entry| entry.delivered && entry.message_id.as_deref() == Some(&message)));
            if !retained { self.generations.remove(&(chat, message)); }
        }
        let expired: Vec<_> = self.legacy_receipts.iter()
            .filter(|(_, at)| at.elapsed() >= std::time::Duration::from_secs(3))
            .map(|(key, _)| key.clone()).collect();
        for (chat, message) in expired {
            let mut completed = Vec::new();
            if let Some(entries) = self.store.orchestrators.get_mut(&chat) {
                entries.retain(|entry| {
                    let prune = entry.delivered && entry.retired_messages.is_empty() && entry.message_id.as_deref() == Some(&message);
                    if prune { completed.push(entry.event.id.clone()); }
                    !prune
                });
            }
            for id in completed { self.store.remember_completed(&chat, &id); }
            let retained = self.store.orchestrators.get(&chat).is_some_and(|entries| entries.iter().any(|entry| entry.delivered && entry.message_id.as_deref() == Some(&message)));
            if !retained { self.legacy_receipts.remove(&(chat, message)); }
        }
    }
    fn write(&self) -> Result<(), String> {
        self.store.write(&self.path).map_err(|error| error.to_string())
    }
}

impl EventTurns {
    pub(crate) fn note_retired(&self, chat_id: &str, message_id: &str, snapshot: Option<&Arc<crate::chat_persistence::ChatPersistence>>) {
        if let Some(persistence) = lock(&self.persistence).as_mut() {
            if let Some(snapshot) = snapshot {
                persistence.retired_generations.insert((chat_id.to_owned(), message_id.to_owned()), (snapshot.clone(), snapshot.generation()));
            } else {
                persistence.legacy_retired.insert((chat_id.to_owned(), message_id.to_owned()), std::time::Instant::now());
            }
        }
    }
    fn synchronize_held(&self, held: &HashMap<String, Vec<WakeEvent>>) -> Result<(), String> {
        if let Some(persistence) = lock(&self.persistence).as_mut() {
            persistence.prune();
            let mut effective = held.clone();
            for chat in &persistence.unavailable {
                if let Some(entries) = persistence.store.orchestrators.get(chat) {
                    effective.entry(chat.clone()).or_default().extend(entries.iter().filter(|entry| !entry.discarded && entry.message_id.is_none()).map(|entry| entry.event.clone()));
                }
            }
            persistence.store.synchronize_held(&effective);
            persistence.write()?;
        }
        Ok(())
    }

    fn mark_queued(&self, chat_id: &str, events: &[WakeEvent], message_id: &str) -> Result<(), String> {
        if let Some(persistence) = lock(&self.persistence).as_mut() {
            persistence.prune();
            persistence.store.mark_queued(chat_id, &events.iter().map(|event| event.id.clone()).collect(), message_id);
            persistence.write()?;
        }
        Ok(())
    }

    pub(crate) fn mark_delivered(self: &Arc<Self>, chat_id: &str, message_id: &str, snapshot: Option<&Arc<crate::chat_persistence::ChatPersistence>>) {
        if let Some(persistence) = lock(&self.persistence).as_mut() {
            let exists = persistence.store.orchestrators.get(chat_id).is_some_and(|events| events.iter().any(|entry| entry.message_id.as_deref() == Some(message_id)));
            if !exists { return; }
            persistence.store.mark_delivered(chat_id, message_id);
            if let Some(snapshot) = snapshot {
                persistence.generations.insert((chat_id.to_owned(), message_id.to_owned()), (snapshot.clone(), snapshot.generation()));
            } else {
                persistence.legacy_receipts.insert((chat_id.to_owned(), message_id.to_owned()), std::time::Instant::now());
            }
            if let Err(error) = persistence.write() {
                tracing::warn!(%error, chat = %chat_id, "could not save wake delivery receipt");
            }
        }
        self.schedule_prune(chat_id, message_id);
    }

    pub(crate) fn schedule_prune(self: &Arc<Self>, chat_id: &str, message_id: &str) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        let turns = Arc::downgrade(self);
        let chat = chat_id.to_owned();
        let message = message_id.to_owned();
        runtime.spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let Some(turns) = turns.upgrade() else { return };
                let chat = chat.clone();
                let message = message.clone();
                let done = tokio::task::spawn_blocking(move || turns.prune_delivered(&chat, &message)).await;
                if !matches!(done, Ok(false)) { return; }
            }
        });
    }

    fn prune_delivered(&self, chat_id: &str, message_id: &str) -> bool {
        let mut guard = lock(&self.persistence);
        let Some(persistence) = guard.as_mut() else { return true };
        let before = persistence.clone();
        persistence.prune();
        let pending = persistence.store.orchestrators.get(chat_id).is_some_and(|entries| entries.iter().any(|entry| entry.message_id.as_deref() == Some(message_id) || entry.retired_messages.iter().any(|retired| retired.chat_id == chat_id && retired.message_id == message_id)));
        if persistence.store == before.store { return !pending; }
        if let Err(error) = persistence.write() {
            tracing::warn!(%error, chat = %chat_id, "could not prune wake delivery receipt");
            *persistence = before;
            return false;
        }
        !pending
    }
    pub fn register(&self, chat_id: &str, message_id: String, events: Vec<WakeEvent>) {
        lock(&self.turns).insert(
            chat_id.to_owned(),
            EventTurn {
                message_id,
                events,
                dispatched: false,
                started: false,
            },
        );
    }

    pub(crate) fn is_discarded_message(&self, chat_id: &str, message_id: &str) -> bool {
        lock(&self.persistence).as_ref().is_some_and(|persistence| {
            persistence.store.orchestrators.get(chat_id).is_some_and(|entries| {
                entries.iter().any(|entry| entry.discarded && entry.message_id.as_deref() == Some(message_id))
            })
        })
    }

    pub(crate) fn discard_queued_message(&self, chat_id: &str, message_id: &str, snapshot: Option<&Arc<crate::chat_persistence::ChatPersistence>>) -> Result<(), String> {
        let mut guard = lock(&self.persistence);
        if let Some(persistence) = guard.as_mut() {
            let before = persistence.clone();
            if let Some(entries) = persistence.store.orchestrators.get_mut(chat_id) {
                for entry in entries.iter_mut().filter(|entry| entry.message_id.as_deref() == Some(message_id)) {
                    entry.discarded = true;
                }
            }
            // Persist the discard before retiring the queue identity.
            if let Err(error) = persistence.write() {
                *persistence = before;
                return Err(error);
            }
            if let Some(entries) = persistence.store.orchestrators.get_mut(chat_id) {
                for entry in entries.iter_mut().filter(|entry| entry.message_id.as_deref() == Some(message_id)) {
                    entry.retire(chat_id, message_id);
                    entry.message_id = None;
                }
            }
            if let Some(snapshot) = snapshot {
                persistence.retired_generations.insert((chat_id.to_owned(), message_id.to_owned()), (snapshot.clone(), snapshot.generation()));
            } else {
                persistence.legacy_retired.insert((chat_id.to_owned(), message_id.to_owned()), std::time::Instant::now());
            }
            if let Err(error) = persistence.write() {
                // The first write remains a valid discard receipt for retry.
                *persistence = before;
                if let Some(entries) = persistence.store.orchestrators.get_mut(chat_id) {
                    for entry in entries.iter_mut().filter(|entry| entry.message_id.as_deref() == Some(message_id)) { entry.discarded = true; }
                }
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn is_event_message(&self, message_id: &str) -> bool {
        lock(&self.turns)
            .values()
            .any(|t| t.message_id == message_id)
    }

    /// The queue row id of a wake that has not been taken yet.
    pub fn pending_message(&self, chat_id: &str) -> Option<String> {
        lock(&self.turns)
            .get(chat_id)
            .filter(|t| !t.dispatched && !t.started)
            .map(|t| t.message_id.clone())
    }

    /// The queue row was pulled back before it ran: its events go back to
    /// the wake service.
    pub fn cancel_pending(&self, chat_id: &str) {
        if let Some(turn) = lock(&self.turns).remove(chat_id) {
            lock(&self.cancelled)
                .entry(chat_id.to_owned())
                .or_default()
                .extend(turn.events);
        }
    }

    pub fn take_cancelled(&self, chat_id: &str) -> Vec<WakeEvent> {
        lock(&self.cancelled).remove(chat_id).unwrap_or_default()
    }

    pub fn mark_dispatched(&self, chat_id: &str) {
        if let Some(turn) = lock(&self.turns).get_mut(chat_id) {
            turn.dispatched = true;
        }
    }

    /// An event turn is running (or about to).
    pub fn has_started(&self, chat_id: &str) -> bool {
        lock(&self.turns)
            .get(chat_id)
            .is_some_and(|t| t.dispatched || t.started)
    }

    pub fn clear(&self, chat_id: &str) {
        lock(&self.turns).remove(chat_id);
    }

    /// Follow the orchestrator's turn: seen in flight → started; finished
    /// after that → no longer an event turn.
    fn observe(&self, chat_id: &str, in_flight: bool) {
        let mut turns = lock(&self.turns);
        let Some(turn) = turns.get_mut(chat_id) else {
            return;
        };
        if in_flight {
            turn.started = true;
        } else if turn.started {
            turns.remove(chat_id);
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ChatInfo {
    title: Option<String>,
    ticket: Option<String>,
    pr_url: Option<String>,
    parent: Option<String>,
    archived: bool,
    last_active: i64,
}

struct TurnStart {
    last_message_id: Option<String>,
    last_completed_turn: Option<String>,
    user_sent_during_turn: bool,
}

#[derive(Default)]
struct State {
    core: WakeCore,
    auto_continue: auto_continue::AutoContinue,
    turn_starts: HashMap<String, TurnStart>,
    auto_continue_default: bool,
    settings_overrides: HashMap<String, WakeSettings>,
    handed_off_to: HashMap<String, String>,
    hand_off_pending: HashSet<String>,
    chats: HashMap<String, ChatInfo>,
    chats_seeded: bool,
    /// Per Space: worker chat → orchestrators by project worker destination.
    destination_parents: HashMap<String, HashMap<String, Vec<String>>>,
    /// Required Action request id → open (unresolved) when last seen.
    requests: HashMap<String, bool>,
    seeded_spaces: HashSet<String>,
    statuses: HashMap<String, SessionStatus>,
    sessions_seeded: bool,
    timers: HashMap<String, i64>,
}

struct Inner {
    state: Mutex<State>,
    sessions: SessionsEngine,
    doc_host: DocHost,
    workspace: WorkspaceHost,
    event_turns: Arc<EventTurns>,
    clock: Clock,
    store: Option<PathBuf>,
    /// Ends the watchers: they hold the sessions/doc-host graph alive.
    cancel: tokio_util::sync::CancellationToken,
}

#[derive(Clone)]
pub struct OrchestratorWakes {
    inner: Arc<Inner>,
}

impl OrchestratorWakes {
    /// Build the service without starting its watchers (tests drive the
    /// `ingest_*` entry points directly).
    pub fn new(
        sessions: SessionsEngine,
        doc_host: DocHost,
        workspace: WorkspaceHost,
        store: Option<PathBuf>,
        clock: Clock,
    ) -> Self {
        let event_turns = doc_host.event_turns();
        let mut state = State::default();
        if let Some(path) = &store
            && let Ok(bytes) = std::fs::read(path)
        {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
                && let Some(saved) = value.get("autoContinueWorkers")
                && let Ok(saved) = serde_json::from_value(saved.clone())
            {
                state.auto_continue = saved;
            }
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
                && let Some(saved) = value.get("handedOffTo")
                && let Ok(saved) = serde_json::from_value(saved.clone())
            {
                state.handed_off_to = saved;
            }
            let (auto_continue_default, saved) = read_settings(&bytes);
            state.auto_continue_default = auto_continue_default;
            for (chat, settings) in saved {
                state.core.set_settings(&chat, settings);
                state.settings_overrides.insert(chat, settings);
            }
        }
        if let Some(settings_path) = &store {
            let path = settings_path.with_file_name("orchestrator-wake-events.json");
            let mut saved = store::WakeStore::read(&path);
            let mut retired_generations = HashMap::new();
            let mut generations = HashMap::new();
            let mut legacy_retired = HashMap::new();
            let mut legacy_receipts = HashMap::new();
            let mut unavailable = HashSet::new();
            for chat in saved.orchestrators.keys().cloned().collect::<Vec<_>>() {
                let ineligible = workspace.chat(&chat).ok().flatten().is_some_and(|recipient| recipient.archived)
                    || !state.core.settings(&chat).enabled;
                if ineligible {
                    if let Some(entries) = saved.orchestrators.get_mut(&chat) {
                        for entry in entries { entry.discarded = true; }
                    }
                    if let Err(error) = saved.write(&path) {
                        tracing::warn!(%error, %chat, "could not persist wake discard before recovery");
                        unavailable.insert(chat);
                        continue;
                    }
                    tracing::info!(%chat, "discarded wakes for archived or disabled coordinator during recovery");
                }
                let handle = match doc_host.open(&chat) {
                    Ok(handle) => handle,
                    Err(error) => { tracing::warn!(%error, %chat, "could not open wake recipient during recovery"); unavailable.insert(chat); continue; }
                };
                let queue: HashSet<String> = match handle.doc().read_queue() {
                    Ok(queue) => queue.into_iter().map(|item| item.id).collect(),
                    Err(error) => { tracing::warn!(%error, %chat, "could not read wake recipient queue during recovery"); unavailable.insert(chat); continue; }
                };
                let transcript: HashSet<String> = handle.doc().recent_message_ids(50).into_iter().collect();
                let mut removed = HashSet::new();
                let mut cross_blocked = HashSet::new();
                if let Some(events) = saved.orchestrators.get(&chat) {
                    let cross: HashSet<_> = events.iter().flat_map(|entry| entry.retired_messages.iter())
                        .filter(|retired| retired.chat_id != chat).map(|retired| (retired.chat_id.clone(), retired.message_id.clone())).collect();
                    for (source, id) in cross {
                            match doc_host.open(&source).and_then(|source_handle| {
                                let present = source_handle.doc().read_queue()?.iter().any(|item| item.id == id);
                                let removed = source_handle.doc().take_queued(&id)?;
                                if !present || removed.is_some() {
                                    if let Some(snapshot) = &source_handle.persistence {
                                        retired_generations.insert((source.clone(), id.clone()), (snapshot.clone(), snapshot.generation()));
                                    } else {
                                        legacy_retired.insert((source.clone(), id.clone()), std::time::Instant::now());
                                    }
                                }
                                Ok((present, removed))
                            }) {
                                Ok((_, Some(_))) => {},
                                Ok((true, None)) => { cross_blocked.insert((source.clone(), id.clone())); },
                                Ok((false, None)) => {},
                                Err(error) => { cross_blocked.insert((source.clone(), id.clone())); tracing::warn!(%error, chat = %source, "could not reclaim transferred wake row"); },
                            }
                    }
                    let ids: HashSet<_> = events.iter().flat_map(|entry| entry.message_id.iter().chain(entry.retired_messages.iter().filter(|retired| retired.chat_id == chat).map(|retired| &retired.message_id))).filter(|id| queue.contains(*id)).cloned().collect();
                    for id in events.iter().filter_map(|entry| entry.message_id.as_ref().filter(|id| entry.discarded || !transcript.contains(*id))).filter(|id| !queue.contains(*id)) {
                        if let Some(snapshot) = &handle.persistence {
                            retired_generations.insert((chat.clone(), id.clone()), (snapshot.clone(), snapshot.generation()));
                        } else {
                            legacy_retired.insert((chat.clone(), id.clone()), std::time::Instant::now());
                        }
                    }
                    for retired in events.iter().flat_map(|entry| entry.retired_messages.iter()).filter(|retired| retired.chat_id == chat && !queue.contains(&retired.message_id)) {
                        if let Some(snapshot) = &handle.persistence {
                            retired_generations.insert((chat.clone(), retired.message_id.clone()), (snapshot.clone(), snapshot.generation()));
                        } else {
                            legacy_retired.insert((chat.clone(), retired.message_id.clone()), std::time::Instant::now());
                        }
                    }
                    for id in ids {
                        match handle.doc().take_queued(&id) {
                            Ok(Some(_)) => {
                                if let Some(snapshot) = &handle.persistence {
                                    retired_generations.insert((chat.clone(), id.clone()), (snapshot.clone(), snapshot.generation()));
                                } else {
                                    legacy_retired.insert((chat.clone(), id.clone()), std::time::Instant::now());
                                }
                                removed.insert(id);
                            },
                            Ok(None) => {},
                            Err(error) => tracing::warn!(%error, chat = %chat, "could not reclaim wake row"),
                        }
                    }
                }
                let mut held = Vec::new();
                let mut completed = Vec::new();
                let mut blocked = false;
                if let Some(events) = saved.orchestrators.get_mut(&chat) {
                    events.retain_mut(|entry| {
                        if entry.retired_messages.iter().any(|retired| cross_blocked.contains(&(retired.chat_id.clone(), retired.message_id.clone()))) { blocked = true; return true; }
                        if entry.discarded {
                            if let Some(id) = entry.message_id.clone() {
                                if queue.contains(&id) && !removed.contains(&id) { blocked = true; return true; }
                                entry.retire(&chat, &id);
                                entry.message_id = None;
                            }
                            if entry.retired_messages.iter().any(|retired| retired.chat_id == chat && queue.contains(&retired.message_id) && !removed.contains(&retired.message_id)) { blocked = true; }
                            return !entry.retired_messages.is_empty();
                        }
                        if let Some(id) = entry.message_id.clone() {
                            if transcript.contains(&id) {
                                let snapshot_pending = handle.persistence.as_ref().is_none_or(|snapshot| !snapshot.has_saved(snapshot.generation()));
                                if removed.contains(&id) || !entry.retired_messages.is_empty() || snapshot_pending {
                                    entry.delivered = true;
                                    if removed.contains(&id) { entry.retire(&chat, &id); }
                                    return true;
                                }
                                completed.push(entry.event.id.clone());
                                return false;
                            }
                            if queue.contains(&id) && !removed.contains(&id) { blocked = true; return true; }
                            entry.retire(&chat, &id);
                            entry.message_id = None;
                            entry.delivered = false;
                        }
                        if entry.retired_messages.iter().any(|retired| retired.chat_id == chat && queue.contains(&retired.message_id) && !removed.contains(&retired.message_id)) { blocked = true; return true; }
                        held.push(entry.event.clone());
                        true
                    });
                }
                if blocked { unavailable.insert(chat.clone()); }
                for id in completed { saved.remember_completed(&chat, &id); }
                if !held.is_empty() { state.core.note_user_activity(&chat, clock(), None); }
                state.core.restore_for_hand_off(&chat, held, clock());
                if let (Some(snapshot), Some(events)) = (&handle.persistence, saved.orchestrators.get(&chat)) {
                    for entry in events.iter().filter(|entry| entry.delivered) {
                        if let Some(id) = &entry.message_id {
                            generations.insert((chat.clone(), id.clone()), (snapshot.clone(), snapshot.generation()));
                        }
                    }
                }
                if handle.persistence.is_none() && let Some(events) = saved.orchestrators.get(&chat) {
                    for entry in events.iter().filter(|entry| entry.delivered) {
                        if let Some(id) = &entry.message_id { legacy_receipts.insert((chat.clone(), id.clone()), std::time::Instant::now()); }
                    }
                }
            }
            for (chat, ids) in &saved.completed_ids {
                for id in ids { state.core.remember_delivered_id(chat, id, clock()); }
            }
            let result = std::thread::scope(|scope| scope.spawn(|| saved.write(&path)).join());
            if let Ok(Err(error)) = result {
                tracing::warn!(%error, "could not save restored wakes");
            }
            *lock(&event_turns.persistence) = Some(WakePersistence { path, store: saved, generations, retired_generations, legacy_receipts, legacy_retired, unavailable });
            let pending: Vec<_> = lock(&event_turns.persistence).as_ref().map(|p| p.generations.keys().chain(p.legacy_receipts.keys()).chain(p.retired_generations.keys()).chain(p.legacy_retired.keys()).cloned().collect()).unwrap_or_default();
            for (chat, message) in pending { event_turns.schedule_prune(&chat, &message); }
        }
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(state),
                sessions,
                doc_host,
                workspace,
                event_turns,
                clock,
                store,
                cancel: tokio_util::sync::CancellationToken::new(),
            }),
        }
    }

    /// Build the service and start its event-driven watchers.
    pub fn start(
        sessions: SessionsEngine,
        doc_host: DocHost,
        workspace: WorkspaceHost,
        store: Option<PathBuf>,
    ) -> Self {
        let this = Self::new(sessions, doc_host, workspace, store, system_clock());
        this.spawn_watchers();
        this
    }

    /// Stop the watchers so a retired engine graph can be freed.
    pub fn shutdown(&self) {
        self.inner.cancel.cancel();
    }

    /// Prune receipts after the document host's graceful snapshot flush.
    pub async fn flush_receipts(&self) {
        let turns = self.inner.event_turns.clone();
        if let Err(error) = tokio::task::spawn_blocking(move || {
            let mut guard = lock(&turns.persistence);
            if let Some(persistence) = guard.as_mut() {
                let before = persistence.clone();
                persistence.prune();
                if persistence.store != before.store && let Err(error) = persistence.write() {
                    *persistence = before;
                    tracing::warn!(%error, "could not save wake receipts during shutdown");
                }
            }
        }).await {
            tracing::warn!(%error, "wake receipt shutdown worker failed");
        }
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    pub(crate) fn spawn_watchers(&self) {
        // The chat watch coalesces rapid changes. Discard while the chat is
        // still archived, before a local unarchive can hide that transition.
        let weak = Arc::downgrade(&self.inner);
        self.inner.workspace.set_before_chat_unarchive(Arc::new(move |chat_id| {
            if let Some(inner) = weak.upgrade() {
                let wakes = OrchestratorWakes { inner };
                wakes.evaluate_blocking(chat_id, false);
                let state = lock(&wakes.inner.state);
                let pending_discard = lock(&wakes.inner.event_turns.persistence).as_ref()
                    .and_then(|persistence| persistence.store.orchestrators.get(chat_id))
                    .is_some_and(|entries| entries.iter().any(|entry| !entry.discarded));
                if state.core.has_held(chat_id) || pending_discard {
                    return Err("could not discard archived wake events before unarchiving".into());
                }
            }
            Ok(())
        }));
        let mut signals = self.inner.sessions.watch_worker_signals();
        let this = self.clone();
        tokio::spawn(async move {
            if let Ok(chats) = this.inner.workspace.read_chats() {
                this.ingest_chats(&chats).await;
            }
            loop {
                tokio::select! {
                    _ = this.inner.cancel.cancelled() => break,
                    signal = signals.recv() => match signal {
                        Ok(signal) => this.ingest_worker_signal(signal).await,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!(n, "worker wake signals dropped"),
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            let mut rx = this.inner.sessions.watch_sessions();
            loop {
                let sessions = rx.borrow_and_update().clone();
                this.ingest_sessions(&sessions).await;
                if this.watch_ended(rx.changed()).await {
                    break;
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            let mut rx = this.inner.workspace.watch_chats();
            loop {
                let chats = rx.borrow_and_update().clone();
                this.ingest_chats(&chats).await;
                if this.watch_ended(rx.changed()).await {
                    break;
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                let worker = this.clone();
                let _ = tokio::task::spawn_blocking(move || worker.notify_due_waiting_items()).await;
                if this.watch_ended(tokio::time::sleep(std::time::Duration::from_secs(30)).map(Ok::<(), ()>)).await {
                    break;
                }
            }
        });
        let this = self.clone();
        tokio::spawn(async move {
            let mut changes = this.inner.workspace.watch_changes();
            let mut markers: HashMap<String, (u64, u64, bool)> = HashMap::new();
            loop {
                if this
                    .watch_ended(
                        tokio::time::sleep(std::time::Duration::from_millis(500)).map(Ok::<(), ()>),
                    )
                    .await
                {
                    break;
                }
                changes.borrow_and_update();
                let spaces = this.inner.workspace.read_spaces().unwrap_or_default();
                for space in spaces {
                    let marker = this.inner.workspace.work_outline_inputs_marker(&space.id);
                    if markers.get(&space.id) == Some(&marker) {
                        continue;
                    }
                    let workspace = this.inner.workspace.clone();
                    let space_id = space.id.clone();
                    let Ok((marker, snapshot)) = tokio::task::spawn_blocking(move || {
                        let (marker, inputs) = workspace.work_outline_inputs_marked(&space_id);
                        (
                            marker,
                            mast_doc::materialize_work_outline(&space_id, &inputs),
                        )
                    })
                    .await
                    else {
                        continue;
                    };
                    markers.insert(space.id.clone(), marker);
                    this.ingest_snapshot(&snapshot).await;
                }
                if this.watch_ended(changes.changed()).await {
                    break;
                }
            }
        });
    }

    #[doc(hidden)]
    pub fn notify_due_waiting_items(&self) {
        use mast_proto::work_outline_user_actions::{UserActionKind, UserActionPriority};
        use mast_proto::{CreateUserActionRequestParams, UserActionOriginKind, UserActionRequestOrigin, WorkOutlineActor, WorkOutlineMutation};
        let workspace = &self.inner.workspace;
        let device_id = workspace.device_id();
        let actor = WorkOutlineActor { kind: WorkOutlineActorKind::System, device_id: device_id.to_owned(), chat_id: None, message_id: None };
        for space in workspace.read_spaces().unwrap_or_default() {
            if space.device_id != device_id { continue; }
            let inputs = workspace.work_outline_inputs(&space.id);
            let Some(snapshot) = materialize_due_outline(&space.id, &inputs, self.now(), || {
                mast_doc::materialize_work_outline(&space.id, &inputs)
            }) else { continue; };
            if !snapshot.available { continue; }
            for item in snapshot.items.iter().filter(|item| item.archived_at.is_none() && item.status != mast_proto::WorkItemStatus::Completed) {
                let Some(until) = item.waiting_until.filter(|until| *until <= self.now()) else { continue; };
                let operation_id = mast_proto::work_outline::derive_id(&format!("{}\0{}\0{}", space.id, item.id, until), "waiting-due");
                let reason = item.waiting_reason.as_deref().filter(|reason| !reason.trim().is_empty()).unwrap_or("ready to review");
                let mutation = WorkOutlineMutation::CreateUserActionRequest(CreateUserActionRequestParams {
                    options: vec![], links: vec![], facts: vec![],
                    kind: Some(UserActionKind::Notice), priority: Some(UserActionPriority::Fyi),
                    work_item_id: item.id.clone(),
                    origin: UserActionRequestOrigin { kind: UserActionOriginKind::System, ..Default::default() },
                    text: format!("Waiting period ended: {}", reason.chars().take(150).collect::<String>()),
                    info: None, dedupe_key: Some(operation_id.clone()), operation_id,
                });
                let _ = workspace.work_outline_mutate(&mutation, &actor, &[]);
            }
        }
    }

    /// Wait for `next`; true when the watch closed or the service shut down.
    async fn watch_ended<T, E>(
        &self,
        next: impl std::future::Future<Output = Result<T, E>>,
    ) -> bool {
        tokio::select! {
            _ = self.inner.cancel.cancelled() => true,
            result = next => result.is_err(),
        }
    }

    // ── settings / status / user activity ─────────────────────────────────

    pub fn settings(&self, chat_id: &str) -> WakeSettings {
        let state = lock(&self.inner.state);
        state
            .settings_overrides
            .get(chat_id)
            .copied()
            .unwrap_or_else(|| WakeSettings {
                auto_continue: state.auto_continue_default,
                ..WakeSettings::default()
            })
    }

    pub async fn set_settings(&self, chat_id: &str, settings: WakeSettings) -> WakeStatus {
        let this = self.clone();
        let chat = chat_id.to_owned();
        if let Err(error) = tokio::task::spawn_blocking(move || {
            let mut state = lock(&this.inner.state);
            state.core.set_settings(&chat, settings);
            state
                .settings_overrides
                .insert(chat, settings);
            if let Err(error) = this.inner.event_turns.synchronize_held(&state.core.held_events()) {
                tracing::warn!(%error, "could not save held wake events");
            }
            this.persist(&state);
        }).await {
            tracing::warn!(%error, "wake settings worker failed");
        }
        self.evaluate(chat_id, false).await;
        self.status(chat_id)
    }

    /// Reclaim a queued wake and transfer its events and effective settings.
    /// Call under the wake state lock, serialized with wake delivery.
    fn transfer_wakes(&self, state: &mut State, from: &str, to: &str) {
        self.inner.doc_host.pull_pending_event_message(from);
        let cancelled = self.inner.event_turns.take_cancelled(from);
        state.core.restore_for_hand_off(from, cancelled, self.now());
        let settings = state.settings_overrides.get(from).copied().unwrap_or_else(|| WakeSettings {
            auto_continue: state.auto_continue_default,
            ..WakeSettings::default()
        });
        state.core.set_settings_preserving_held(to, settings);
        state.settings_overrides.insert(to.to_owned(), settings);
        let held_ids: HashSet<String> = state.core.held_events().remove(from).unwrap_or_default().into_iter().map(|event| event.id).collect();
        state.core.transfer_held(from, to);
        if let Some(persistence) = lock(&self.inner.event_turns.persistence).as_mut() {
            persistence.store.transfer(from, to, &held_ids);
        }
        state.handed_off_to.insert(from.to_owned(), to.to_owned());
        state.timers.remove(from);
    }

    /// Hand off direct workers and their pending wakes to another orchestrator.
    pub async fn hand_off_orchestrator(&self, from: &str, to: &str, actor: &str) -> Result<Vec<String>, String> {
        let workers = self.hand_off_orchestrator_without_delivery(from, to, actor).await?;
        self.finish_orchestrator_hand_off(to);
        self.evaluate(to, false).await;
        Ok(workers)
    }

    pub(crate) async fn hand_off_orchestrator_without_delivery(&self, from: &str, to: &str, actor: &str) -> Result<Vec<String>, String> {
        let this = self.clone();
        let (from, to, actor) = (from.to_owned(), to.to_owned(), actor.to_owned());
        tokio::task::spawn_blocking(move || this.hand_off_orchestrator_blocking(&from, &to, &actor)).await
            .map_err(|error| format!("wake hand-off worker failed: {error}"))?
    }

    fn hand_off_orchestrator_blocking(&self, from: &str, to: &str, actor: &str) -> Result<Vec<String>, String> {
        let workers = {
            let mut state = lock(&self.inner.state);
            if lock(&self.inner.event_turns.persistence).as_ref().is_some_and(|persistence| persistence.unavailable.contains(from)) {
                return Err("wake recovery is blocked for the source chat; restart and retry the hand-off".into());
            }
            if state.handed_off_to.get(from).is_some_and(|existing| existing != to) {
                return Err("source already handed off to another chat".into());
            }
            let mut cursor = to;
            let mut remaining = state.handed_off_to.len() + 1;
            while let Some(next) = state.handed_off_to.get(cursor) {
                if next == from || remaining == 0 {
                    return Err("hand-off would create a routing cycle".into());
                }
                cursor = next;
                remaining -= 1;
            }
            let (workers, replayed) = self.inner.workspace.hand_off_chat_children(from, to, actor, self.now())?;
            state.hand_off_pending.insert(to.to_owned());
            // Registry and wake settings use separate stores. If a process
            // stopped after the registry write, complete the transfer on retry.
            if !replayed || state.handed_off_to.get(from).map(String::as_str) != Some(to) {
                self.transfer_wakes(&mut state, from, to);
                for worker in &workers {
                    if let Some(info) = state.chats.get_mut(worker) {
                        info.parent = Some(to.to_owned());
                    }
                }
            }
            if let Some(persistence) = lock(&self.inner.event_turns.persistence).as_mut() {
                persistence.write().map_err(|error| format!("could not save transferred wake events: {error}"))?;
            }
            self.persist_checked(&state).map_err(|error| format!("could not save wake settings: {error}"))?;
            workers
        };
        Ok(workers)
    }

    pub(crate) fn finish_orchestrator_hand_off(&self, to: &str) {
        lock(&self.inner.state).hand_off_pending.remove(to);
    }

    fn persist(&self, state: &State) {
        if let Err(error) = self.persist_checked(state) {
            tracing::warn!(%error, "could not save orchestrator wake settings");
        }
    }

    fn persist_checked(&self, state: &State) -> Result<(), std::io::Error> {
        let Some(path) = &self.inner.store else {
            return Ok(());
        };
        let settings = serde_json::json!({
            "autoContinue": state.auto_continue_default,
            "orchestrators": state.settings_overrides,
            "autoContinueWorkers": state.auto_continue,
            "handedOffTo": state.handed_off_to,
        });
        let bytes = serde_json::to_vec_pretty(&settings).map_err(std::io::Error::other)?;
        let staging = path.with_extension("handoff-tmp");
        let mut file = std::fs::File::create(&staging)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()?;
        std::fs::rename(staging, path)?;
        Ok(())
    }

    pub fn status(&self, chat_id: &str) -> WakeStatus {
        let ctx = self.ctx(chat_id);
        let now = self.now();
        let mut state = lock(&self.inner.state);
        let mut status = state.core.status(chat_id, now, ctx);
        status.settings = state
            .settings_overrides
            .get(chat_id)
            .copied()
            .unwrap_or_else(|| WakeSettings {
                auto_continue: state.auto_continue_default,
                ..WakeSettings::default()
            });
        status.is_orchestrator = is_orchestrator(&state, chat_id);
        status
    }

    /// The user is focused / typing / has a draft in `chat_id`.
    pub async fn note_user_activity(&self, chat_id: &str, draft_non_empty: Option<bool>) {
        let now = self.now();
        lock(&self.inner.state)
            .core
            .note_user_activity(chat_id, now, draft_non_empty);
        self.evaluate(chat_id, false).await;
    }

    /// "Deliver now": skips the window and the user-activity hold, never a
    /// running turn or a queued user message.
    pub async fn deliver_now(&self, chat_id: &str) -> WakeStatus {
        self.evaluate(chat_id, true).await;
        self.status(chat_id)
    }

    // ── ingestion (event driven) ──────────────────────────────────────────

    /// Consume the input bridge and end-of-turn signals, without relying on a
    /// coalescing session watch to observe every prompt or completed turn.
    pub async fn ingest_worker_signal(&self, signal: crate::sessions::WorkerSignal) {
        use crate::sessions::WorkerSignal;
        // A newly assigned worker can ask before either workspace watcher
        // catches up. Refresh its Space so project-destination orchestrators
        // receive the event as well as its direct parent.
        let worker = match &signal {
            WorkerSignal::Input { chat_id, .. } | WorkerSignal::ToolFailures { chat_id, .. } => {
                chat_id
            }
        };
        if let Ok(chats) = self.inner.workspace.read_chats() {
            let space_id = chats
                .iter()
                .find(|chat| &chat.id == worker)
                .and_then(|chat| chat.space_id.clone());
            self.ingest_chats(&chats).await;
            if let Some(space_id) = space_id {
                let workspace = self.inner.workspace.clone();
                if let Ok(snapshot) = tokio::task::spawn_blocking(move || {
                    let inputs = workspace.work_outline_inputs(&space_id);
                    mast_doc::materialize_work_outline(&space_id, &inputs)
                })
                .await
                {
                    self.ingest_snapshot(&snapshot).await;
                }
            }
        }
        let now = self.now();
        let (worker, kind, detail, key) = match signal {
            WorkerSignal::Input { chat_id, input } => {
                let question = input
                    .questions
                    .iter()
                    .map(|q| q.question.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                let approval = input.questions.iter().any(|q| {
                    let header = q.header.to_lowercase();
                    header.contains("approval") || header.contains("permission")
                });
                let kind = if approval { "approval" } else { "input" };
                let detail = format!("is waiting for {kind}: {}", truncate(&question, 300));
                let key = format!("input:{}:{}", chat_id, input.request_id);
                (chat_id, WakeEventKind::AwaitingInput, detail, key)
            }
            WorkerSignal::ToolFailures { chat_id, failures } => {
                let detail = format!(
                    "turn ended after {} failed Mast tool call{} ({})",
                    failures.count,
                    if failures.count == 1 { "" } else { "s" },
                    failures.tools.join(", ")
                );
                let key = format!("tool-fail:{}:{}", chat_id, uuid::Uuid::new_v4());
                (chat_id, WakeEventKind::ToolCallsFailed, detail, key)
            }
        };
        let mut recorded = HashSet::new();
        {
            let mut state = lock(&self.inner.state);
            let line = format!("{} — {detail}", label(&state, &worker));
            for orch in parents_of(&state, &worker) {
                let event = make_event(&mut state, &worker, kind, key.clone(), &line, now);
                state.core.record(&orch, event, now);
                recorded.insert(orch);
            }
        }
        self.after_ingest(recorded).await;
    }

    /// Session transitions of workers (errored, turn finished) and of
    /// orchestrators (turn start/end re-evaluates held events).
    pub async fn ingest_sessions(&self, sessions: &[Session]) {
        let now = self.now();
        let mut recorded: HashSet<String> = HashSet::new();
        let mut finished = Vec::new();
        // Recovery may read a journal once; never do that under the wake lock.
        let limited_sessions: HashSet<String> = sessions.iter()
            .filter(|session| session.status == SessionStatus::Errored)
            .filter(|session| self.inner.sessions.usage_limit_reset(&session.chat_id).is_some_and(|at| at > now))
            .map(|session| session.chat_id.clone()).collect();
        {
            let mut state = lock(&self.inner.state);
            let seeded = state.sessions_seeded;
            for session in sessions {
                let prev = state
                    .statuses
                    .insert(session.chat_id.clone(), session.status);
                let in_flight = matches!(
                    session.status,
                    SessionStatus::Working | SessionStatus::AwaitingInput
                );
                self.inner.event_turns.observe(&session.chat_id, in_flight);
                if in_flight && prev.is_none_or(|p| !is_in_flight(p)) {
                    state.core.note_turn_start(&session.chat_id, now);
                    if let Some(last_message) = self.last_external_message(&session.chat_id) {
                        let user_sent_during_turn = session.started_at.is_some_and(|started| {
                            last_message
                                .as_ref()
                                .is_some_and(|(_, at)| *at > started.timestamp_millis())
                        });
                        state.turn_starts.insert(
                            session.chat_id.clone(),
                            TurnStart {
                                last_message_id: last_message.map(|(id, _)| id),
                                last_completed_turn: session.last_completed_turn.clone(),
                                user_sent_during_turn,
                            },
                        );
                    }
                }
                if !seeded {
                    continue;
                }
                let Some(prev) = prev else { continue };
                let kind = match (is_in_flight(prev), session.status) {
                    (_, s) if s == prev => continue,
                    (_, SessionStatus::Errored) => WakeEventKind::Errored,
                    (true, SessionStatus::Idle) => WakeEventKind::TurnFinished,
                    _ => continue,
                };
                let limited_error = kind == WakeEventKind::Errored
                    && limited_sessions.contains(&session.chat_id);
                if kind == WakeEventKind::TurnFinished || limited_error {
                    if let Some(start) = state.turn_starts.remove(&session.chat_id) {
                        finished.push((session.clone(), start));
                    }
                } else {
                    state.turn_starts.remove(&session.chat_id);
                }
                let summary = match kind {
                    WakeEventKind::Errored => "errored (read_chat for details)",
                    _ => "finished a turn",
                };
                let key = format!(
                    "{}:{}:{}",
                    kind_tag(kind),
                    session.chat_id,
                    now / WAKE_WINDOW_MS
                );
                let line = format!("{} — {summary}", label(&state, &session.chat_id));
                for orch in parents_of(&state, &session.chat_id) {
                    let event =
                        make_event(&mut state, &session.chat_id, kind, key.clone(), &line, now);
                    state.core.record(&orch, event, now);
                    recorded.insert(orch);
                }
            }
            state.sessions_seeded = true;
        }
        for (session, start) in finished {
            self.continue_worker(&session, start).await;
        }
        self.after_ingest(recorded).await;
    }

    /// The latest externally submitted message; Mast's system messages are
    /// written as user entries by the queue and do not reset the budget.
    fn last_external_message(&self, chat_id: &str) -> Option<Option<(String, i64)>> {
        let handle = self.inner.doc_host.open(chat_id).ok()?;
        let entries = handle.doc().read_entries().ok()?;
        Some(entries.into_iter().rev().find(|entry| {
            entry.role == mast_doc::MessageRole::User
                && !entry.parts.iter().any(|part| matches!(part,
                    mast_doc::MessagePart::Text { text, .. } if text.starts_with(mast_proto::SYSTEM_MESSAGE_PREFIX)))
        }).map(|entry| (entry.id, entry.created_at)))
    }

    async fn continue_worker(&self, session: &Session, start: TurnStart) {
        let worker = &session.chat_id;
        if let Some(reset) = self.inner.sessions.usage_limit_reset(worker).filter(|at| *at > self.now()) {
            let this = self.clone();
            let session = session.clone();
            let delay = (reset - self.now()) as u64;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                if let Some(current) = this.inner.sessions.session_status(&session.chat_id)
                    && current.status == session.status
                    && current.updated_at == session.updated_at
                    && current.last_completed_turn == session.last_completed_turn
                    && this.inner.sessions.usage_limit_reset(&session.chat_id).is_none_or(|at| at <= this.now())
                {
                    this.continue_worker_ready(&current, start, true).await;
                }
            });
            return;
        }
        self.continue_worker_ready(session, start, false).await;
    }

    async fn continue_worker_ready(&self, session: &Session, start: TurnStart, limited: bool) {
        let worker = &session.chat_id;
        if !self.hosted_here(worker) || (session.status != SessionStatus::Idle
            && !(limited && session.status == SessionStatus::Errored)) {
            return;
        }
        let Ok(chats) = self.inner.workspace.read_chats() else {
            return;
        };
        let Some(space_id) = chats
            .iter()
            .find(|chat| &chat.id == worker)
            .and_then(|chat| chat.space_id.clone())
        else {
            return;
        };
        self.ingest_chats(&chats).await;
        let workspace = self.inner.workspace.clone();
        let Ok(snapshot) = tokio::task::spawn_blocking(move || {
            mast_doc::materialize_work_outline(&space_id, &workspace.work_outline_inputs(&space_id))
        })
        .await
        else {
            return;
        };
        if !snapshot.available {
            return;
        }
        self.ingest_snapshot(&snapshot).await;
        let Some(item) = snapshot
            .items
            .iter()
            .find(|item| item.chat_ids.contains(worker))
        else {
            return;
        };
        let Some(last_message) = self.last_external_message(worker) else {
            return;
        };
        let last_message_id = last_message.map(|(id, _)| id);
        let Ok(handle) = self.inner.doc_host.open(worker) else {
            return;
        };
        let Ok(queue) = handle.doc().read_queue() else {
            return;
        };
        let action_open = snapshot
            .user_action_requests
            .iter()
            .any(|request| request.chat_id == *worker && request.is_unresolved());
        let now = self.now();
        let recorded = {
            let mut state = lock(&self.inner.state);
            let Some(orch) = parents_of(&state, worker).into_iter().next() else {
                return;
            };
            let settings = state
                .settings_overrides
                .get(&orch)
                .copied()
                .unwrap_or_else(|| WakeSettings {
                    auto_continue: state.auto_continue_default,
                    ..WakeSettings::default()
                });
            let mut next = state.auto_continue.clone();
            let decision = next.decide(auto_continue::Turn {
                worker_id: worker,
                item_id: &item.id,
                last_message_id: last_message_id.as_deref(),
                enabled: settings.enabled && settings.auto_continue,
                item_open: item.status != mast_proto::WorkItemStatus::Completed
                    && item.archived_at.is_none(),
                item_waiting: item.waiting_until.is_some(),
                action_open,
                completed: limited || (session.last_completed_turn.is_some()
                    && session.last_completed_turn != start.last_completed_turn),
                awaiting_input: session.awaiting_input.is_some(),
                tool_failures: session.last_turn_tool_failures.is_some(),
                user_sent_during_turn: start.user_sent_during_turn
                    || last_message_id != start.last_message_id,
                queue_nonempty: !queue.is_empty() || self.inner.sessions.turn_in_flight(worker),
            });
            match decision {
                auto_continue::Decision::None => {
                    if state.auto_continue != next {
                        state.auto_continue = next;
                        self.persist(&state);
                    }
                    None
                }
                auto_continue::Decision::Continue(count) => {
                    let text = format!(
                        "[Mast system message, not from the user: auto-continue {count}/3]\nYour work item is still open. Continue the task; if you are done, mark the item completed or raise a Required Action (kind handoff) — if you're blocked, raise one with kind ask."
                    );
                    match self.inner.doc_host.queue_event_message(worker, &text, &crate::new_id()) {
                        Ok(id) => {
                            self.inner.event_turns.register(worker, id, Vec::new());
                            state.auto_continue = next;
                            self.persist(&state);
                        }
                        Err(err) => {
                            tracing::warn!(chat = %worker, error = %err, "worker auto-continue queue failed")
                        }
                    }
                    None
                }
                auto_continue::Decision::ReportStopped => {
                    state.core.set_settings(&orch, settings);
                    let line = format!(
                        "{} — worker stopped with its item open after 3 auto-continues",
                        label(&state, worker)
                    );
                    let event = make_event(
                        &mut state,
                        worker,
                        WakeEventKind::AutoContinueStopped,
                        format!("auto-continue:{worker}:{}", uuid::Uuid::new_v4()),
                        &line,
                        now,
                    );
                    state.core.record(&orch, event, now);
                    state.auto_continue = next;
                    self.persist(&state);
                    Some(orch)
                }
            }
        };
        if let Some(orch) = recorded {
            self.after_ingest(HashSet::from([orch])).await;
        }
    }

    /// Chat rows: parent links, titles, and PR links (PR opened).
    pub async fn ingest_chats(&self, chats: &[Chat]) {
        let now = self.now();
        let mut recorded: HashSet<String> = HashSet::new();
        let mut archived = Vec::new();
        {
            let mut state = lock(&self.inner.state);
            let seeded = state.chats_seeded;
            for chat in chats {
                let info = ChatInfo {
                    title: chat.title.clone(),
                    ticket: firm_link(chat.linked_ticket_id.as_ref(), chat.linked_ticket_source),
                    pr_url: firm_link(chat.linked_pr_url.as_ref(), chat.linked_pr_source),
                    parent: chat.parent_chat_id.clone(),
                    archived: chat.archived,
                    last_active: chat.last_message_at.unwrap_or(chat.created_at).timestamp_millis(),
                };
                let prev = state.chats.insert(chat.id.clone(), info.clone());
                if info.archived && prev.as_ref().is_none_or(|previous| !previous.archived) {
                    archived.push(chat.id.clone());
                }
                if !seeded {
                    continue;
                }
                if let Some(url) = &info.pr_url
                    && prev.as_ref().is_none_or(|p| p.pr_url.as_ref() != Some(url))
                {
                    let line = format!("{} — PR opened: {url}", label(&state, &chat.id));
                    for orch in parents_of(&state, &chat.id) {
                        let event = make_event(
                            &mut state,
                            &chat.id,
                            WakeEventKind::PrOpened,
                            format!("pr:{}:{url}", chat.id),
                            &line,
                            now,
                        );
                        state.core.record(&orch, event, now);
                        recorded.insert(orch);
                    }
                }
            }
            state.chats_seeded = true;
        }
        self.after_ingest(recorded).await;
        for chat in archived {
            self.evaluate(&chat, false).await;
        }
    }

    /// Whether any current orchestrator has enabled PR-merged wakes.
    pub fn accepts_pr_merged(&self, worker_chat_id: &str) -> bool {
        let state = lock(&self.inner.state);
        parents_of(&state, worker_chat_id).iter().any(|orch|
            state.core.settings(orch).wakes_for(WakeEventKind::PrMerged))
    }

    /// Queue a worker's merged PR for its orchestrators.
    pub async fn note_pr_merged(&self, worker_chat_id: &str, pr_url: &str, detail: &str) {
        let now = self.now();
        let mut recorded = HashSet::new();
        {
            let mut state = lock(&self.inner.state);
            let line = format!("{} — PR merged: {detail}", label(&state, worker_chat_id));
            for orch in parents_of(&state, worker_chat_id) {
                let event = make_event(
                    &mut state,
                    worker_chat_id,
                    WakeEventKind::PrMerged,
                    format!("prm:{worker_chat_id}:{pr_url}"),
                    &line,
                    now,
                );
                state.core.record(&orch, event, now);
                recorded.insert(orch);
            }
        }
        self.after_ingest(recorded).await;
    }

    /// A worker's report to its parent chat (`report_to_parent`, or
    /// `send_message` to the parent): queued as a "worker report" event and
    /// delivered in the next "Worker updates:" batch under the usual
    /// idle / user-first rules. Parent only, same Space, and only when this
    /// device hosts the parent (only the host delivers).
    pub async fn report_to_parent(
        &self,
        from: &str,
        text: &str,
    ) -> Result<mast_proto::ReportToParentResult, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("text is empty".into());
        }
        let (parent_id, truncated) = {
        let mut state = lock(&self.inner.state);
        let worker = self
            .inner
            .workspace
            .chat(from)
            .ok()
            .flatten()
            .ok_or_else(|| format!("unknown chat {from}"))?;
        let parent_id = worker
            .parent_chat_id
            .clone()
            .filter(|p| p != from)
            .ok_or("this chat has no parent chat, so there is nobody to report to")?;
        let parent = self
            .inner
            .workspace
            .chat(&parent_id)
            .ok()
            .flatten()
            .ok_or("the parent chat no longer exists")?;
        if parent.archived {
            return Err("the parent chat is archived; hand off or unarchive it before reporting".into());
        }
        if parent.space_id != worker.space_id {
            return Err("the parent chat is in another Space; reports never cross Spaces".into());
        }
        if parent.device_id != self.inner.doc_host.device_id() {
            return Err(
                "the parent chat runs on another device; worker reports are only delivered \
                 by the device that hosts the parent (read_chat / Required Actions still work)"
                    .into(),
            );
        }
        let (report, truncated) = mast_proto::cap_worker_report(text);
        if !state.core.settings(&parent_id).wakes_for(WakeEventKind::WorkerReport) {
            return Ok(mast_proto::ReportToParentResult {
                parent_chat_id: parent_id,
                delivery: "parentReportsOff".into(),
                held_reason: None,
                truncated,
            });
        }
        let now = self.now();
        {
            let info = state.chats.entry(from.to_owned()).or_default();
            info.title = worker.title.clone();
            info.ticket = worker.linked_ticket_id.clone();
            info.parent = Some(parent_id.clone());
            let first = report.lines().next().unwrap_or_default();
            let line = format!(
                "{} — worker report: {}",
                label(&state, from),
                truncate(first, 120)
            );
            let key = format!("rep:{from}:{}", report_hash(&report, now / WAKE_WINDOW_MS));
            let mut event = make_event(
                &mut state,
                from,
                WakeEventKind::WorkerReport,
                key,
                &line,
                now,
            );
            event.report = Some(report);
            state.core.record(&parent_id, event, now);
        }
        (parent_id, truncated)
        };
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.persist_held()).await
            .map_err(|error| format!("wake persistence worker failed: {error}"))??;
        self.evaluate(&parent_id, false).await;
        let status = self.status(&parent_id);
        if !status.settings.wakes_for(WakeEventKind::WorkerReport) {
            return Ok(mast_proto::ReportToParentResult {
                parent_chat_id: parent_id,
                delivery: "parentReportsOff".into(),
                held_reason: None,
                truncated,
            });
        }
        Ok(mast_proto::ReportToParentResult {
            parent_chat_id: parent_id,
            delivery: "queuedForParent".into(),
            held_reason: status.hold_reason,
            truncated,
        })
    }

    /// Required Actions raised/resolved, plus the project-destination
    /// worker relation, from one Space's snapshot.
    pub async fn ingest_snapshot(&self, snapshot: &WorkOutlineSnapshot) {
        if !snapshot.available {
            return;
        }
        let now = self.now();
        let mut recorded: HashSet<String> = HashSet::new();
        {
            let mut state = lock(&self.inner.state);
            state
                .destination_parents
                .insert(snapshot.space_id.clone(), destination_parents(snapshot));
            let seeded = !state.seeded_spaces.insert(snapshot.space_id.clone());
            for request in &snapshot.user_action_requests {
                let open = request.is_unresolved();
                let key = format!(
                    "act:{}:{}",
                    request.work_item_id,
                    request.dedupe_key.as_deref().unwrap_or(&request.text)
                );
                let prev = state.requests.insert(request.id.clone(), open);
                if !seeded || request.chat_id.is_empty() {
                    continue;
                }
                let was_open = prev.unwrap_or(false);
                let (kind, event_key, text) = if open && !was_open {
                    let mut text = request.text.trim().to_owned();
                    if let Some(info) = request.info.as_deref().map(str::trim)
                        && !info.is_empty()
                    {
                        text = format!("{text}: {}", truncate(info, 160));
                    }
                    (WakeEventKind::ActionRaised, key, text)
                } else if !open && was_open {
                    (
                        WakeEventKind::ActionResolved,
                        resolved_key(&key),
                        format!("resolved: {}", request.text.trim()),
                    )
                } else {
                    continue;
                };
                let mut line = format!("{} — {text}", label(&state, &request.chat_id));
                if let Some(pr) = state
                    .chats
                    .get(&request.chat_id)
                    .and_then(|c| c.pr_url.clone())
                {
                    line.push_str(&format!(" (PR {pr})"));
                }
                if kind == WakeEventKind::ActionRaised {
                    line.push_str(&format!(" [request {}]", request.id));
                }
                for orch in parents_of(&state, &request.chat_id) {
                    if kind == WakeEventKind::ActionResolved
                        && request.resolution.as_ref().is_some_and(|resolution| {
                            !resolution.answered_by_user()
                                && resolution.by.kind == WorkOutlineActorKind::Agent
                                && resolution.by.chat_id.as_deref() == Some(orch.as_str())
                        })
                    {
                        continue;
                    }
                    let event = make_event(
                        &mut state,
                        &request.chat_id,
                        kind,
                        event_key.clone(),
                        &line,
                        now,
                    );
                    state.core.record(&orch, event, now);
                    recorded.insert(orch);
                }
            }
        }
        self.after_ingest(recorded).await;
    }

    async fn after_ingest(&self, recorded: HashSet<String>) {
        if !recorded.is_empty() {
            let this = self.clone();
            match tokio::task::spawn_blocking(move || this.persist_held()).await {
                Ok(Ok(())) => {},
                Ok(Err(error)) => tracing::warn!(%error, "could not save held wake events"),
                Err(error) => tracing::warn!(%error, "held wake persistence worker failed"),
            }
        }
        // Orchestrators that recorded something, plus any holding events
        // whose blocker (a running turn) may just have ended.
        let mut targets: HashSet<String> = recorded;
        targets.extend(lock(&self.inner.state).core.orchestrators_with_held());
        for orch in targets {
            self.evaluate(&orch, false).await;
        }
    }

    fn persist_held(&self) -> Result<(), String> {
        let state = lock(&self.inner.state);
        self.inner.event_turns.synchronize_held(&state.core.held_events())
    }

    // ── delivery ──────────────────────────────────────────────────────────

    fn ctx(&self, orch: &str) -> Ctx {
        let queue_nonempty = self
            .inner
            .doc_host
            .open(orch)
            .ok()
            .and_then(|h| h.doc().read_queue().ok())
            .is_some_and(|q| !q.is_empty());
        Ctx {
            busy: self.inner.sessions.turn_in_flight(orch),
            queue_nonempty,
        }
    }

    /// Whether this device hosts `orch` (only the host delivers).
    fn hosted_here(&self, orch: &str) -> bool {
        self.inner
            .workspace
            .chat(orch)
            .ok()
            .flatten()
            .is_some_and(|c| c.device_id == self.inner.doc_host.device_id())
    }

    /// Decide for one orchestrator and act: deliver, arm a one-shot timer, or
    /// keep holding until the next event (turn end, draft cleared).
    pub async fn evaluate(&self, orch: &str, force: bool) {
        let this = self.clone();
        let orch = orch.to_owned();
        if let Err(error) = tokio::task::spawn_blocking(move || this.evaluate_blocking(&orch, force)).await {
            tracing::warn!(%error, "wake evaluation worker failed");
        }
    }

    fn evaluate_blocking(&self, orch: &str, force: bool) {
        let now = self.now();
        let ctx = self.ctx(orch);
        let limit_reset = self.inner.sessions.usage_limit_reset(orch).filter(|at| *at > now);
        // Keep selection and queueing under the same lock: several watchers can
        // evaluate the same orchestrator concurrently.
        let mut state = lock(&self.inner.state);
        let archived = self.inner.workspace.chat(orch).ok().flatten().is_some_and(|chat| chat.archived);
        if archived || !state.core.settings(orch).enabled {
            {
                let mut guard = lock(&self.inner.event_turns.persistence);
                if let Some(persistence) = guard.as_mut() {
                    let before = persistence.store.clone();
                    if let Some(entries) = persistence.store.orchestrators.get_mut(orch) {
                        for entry in entries { entry.discarded = true; }
                    }
                    if let Err(error) = persistence.write() {
                        persistence.store = before;
                        tracing::warn!(%error, chat = %orch, "could not persist wake discard");
                        return;
                    }
                }
            }
            let pending = self.inner.event_turns.pending_message(orch);
            self.inner.doc_host.pull_pending_event_message(orch);
            let _ = self.inner.event_turns.take_cancelled(orch);
            state.core.discard_held(orch);
            state.timers.remove(orch);
            if let Some(persistence) = lock(&self.inner.event_turns.persistence).as_mut() {
                if let Some(entries) = persistence.store.orchestrators.get_mut(orch) {
                    for entry in entries.iter_mut() {
                        if pending.as_ref().is_some_and(|id| entry.message_id.as_ref() == Some(id))
                            && !self.inner.event_turns.has_started(orch)
                            && self.inner.event_turns.pending_message(orch).is_none() {
                            let id = entry.message_id.take().unwrap();
                            entry.retire(orch, &id);
                        }
                    }
                    entries.retain(|entry| entry.message_id.is_some() || !entry.retired_messages.is_empty());
                }
                if let Err(error) = persistence.write() {
                    tracing::warn!(%error, chat = %orch, "could not save wake discard receipt");
                }
            }
            if let Some(id) = pending { self.inner.event_turns.schedule_prune(orch, &id); }
            tracing::info!(chat = %orch, "discarded wakes for archived or disabled coordinator");
            return;
        }
        if state.hand_off_pending.contains(orch) {
            return;
        }
        if let Some(reset) = limit_reset {
            drop(state);
            self.arm_timer(orch, reset);
            return;
        }
        let cancelled = self.inner.event_turns.take_cancelled(orch);
        if !cancelled.is_empty() {
            state.core.restore(orch, cancelled, now);
            if let Err(error) = self.inner.event_turns.synchronize_held(&state.core.held_events()) {
                tracing::warn!(%error, "could not save restored wake events");
                return;
            }
        }
        match state.core.decide(orch, now, ctx, force) {
            Decision::Nothing | Decision::Hold(_) => {}
            Decision::WaitUntil { at_ms, .. } => {
                drop(state);
                self.arm_timer(orch, at_ms);
            }
            Decision::Deliver(batch) => {
                if !self.hosted_here(orch) {
                    return;
                }
                let text = batch.render(|id| {
                    let title = self.inner.workspace.chat(id).ok().flatten().and_then(|chat| chat.title);
                    wake_label(id, title.as_deref())
                });
                let message_id = crate::new_id();
                if let Err(error) = self.inner.event_turns.synchronize_held(&state.core.held_events()) {
                    tracing::warn!(%error, chat = %orch, "could not save held wake events");
                    return;
                }
                if let Err(error) = self.inner.event_turns.mark_queued(orch, &batch.events, &message_id) {
                    tracing::warn!(%error, chat = %orch, "could not save wake queue receipt");
                    return;
                }
                self.inner.event_turns.register(orch, message_id.clone(), batch.events.clone());
                match self.inner.doc_host.queue_event_message(orch, &text, &message_id) {
                    Ok(message_id) => {
                        let _ = message_id;
                        state.core.delivered(orch, &batch, now);
                    }
                    Err(err) => {
                        self.inner.event_turns.clear(orch);
                        if let Err(error) = self.inner.event_turns.synchronize_held(&state.core.held_events()) {
                            tracing::warn!(%error, "could not save held wake events");
                        }
                        tracing::warn!(chat = %orch, error = %err, "orchestrator wake delivery failed");
                    }
                }
            }
        }
    }

    fn arm_timer(&self, orch: &str, at_ms: i64) {
        let now = self.now();
        {
            let mut state = lock(&self.inner.state);
            if state
                .timers
                .get(orch)
                .is_some_and(|&t| t > now && t <= at_ms)
            {
                return; // an earlier timer re-evaluates (and re-arms) first
            }
            state.timers.insert(orch.to_owned(), at_ms);
        }
        let this = self.clone();
        let orch = orch.to_owned();
        let wait = (at_ms - now).max(0) as u64;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            {
                let mut state = lock(&this.inner.state);
                if state.timers.get(&orch) == Some(&at_ms) {
                    state.timers.remove(&orch);
                }
            }
            this.evaluate(&orch, false).await;
        });
    }
}

fn is_in_flight(status: SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Working | SessionStatus::AwaitingInput
    )
}

fn kind_tag(kind: WakeEventKind) -> &'static str {
    match kind {
        WakeEventKind::ActionRaised => "act",
        WakeEventKind::ActionResolved => "res",
        WakeEventKind::Errored => "err",
        WakeEventKind::TurnFinished => "turn",
        WakeEventKind::PrOpened => "pr",
        WakeEventKind::PrMerged => "prm",
        WakeEventKind::WorkerReport => "rep",
        WakeEventKind::AwaitingInput => "input",
        WakeEventKind::ToolCallsFailed => "tool-fail",
        WakeEventKind::AutoContinueStopped => "auto-continue-stopped",
    }
}

fn report_hash(text: &str, bucket: i64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (text, bucket).hash(&mut h);
    h.finish()
}

fn truncate(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    format!("{cut}…")
}

/// A link worth naming a worker by or announcing: anything but a bare
/// `mentioned` one, which only says the chat's text contained the id or URL
/// (a planner discussing ENG-3219 is not working on it, and a PR it read
/// about was not opened by it).
fn firm_link(value: Option<&String>, source: Option<mast_proto::ChatLinkSource>) -> Option<String> {
    match source {
        Some(mast_proto::ChatLinkSource::Mentioned) | None => None,
        Some(_) => value.cloned(),
    }
}

fn wake_label(chat_id: &str, title: Option<&str>) -> String {
    let short: String = chat_id.chars().take(12).collect();
    match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => format!("{title} (chat {short})"),
        None => format!("chat {short}"),
    }
}

/// "ENG-2818" when the worker has a firmly linked ticket, else its title, then the
/// chat id prefix the tools accept.
fn label(state: &State, chat_id: &str) -> String {
    let short: String = chat_id.chars().take(12).collect();
    let info = state.chats.get(chat_id);
    let name = info
        .and_then(|c| c.ticket.clone())
        .or_else(|| info.and_then(|c| c.title.clone()))
        .filter(|n| !n.trim().is_empty());
    match name {
        Some(name) => format!("{} (chat {short})", name.trim()),
        None => format!("chat {short}"),
    }
}

fn make_event(
    state: &mut State,
    worker: &str,
    kind: WakeEventKind,
    key: String,
    summary: &str,
    now: i64,
) -> WakeEvent {
    WakeEvent {
        id: state.core.next_event_id(),
        key,
        worker_chat_id: worker.to_owned(),
        worker_title: state.chats.get(worker).and_then(|c| c.title.clone()),
        kind,
        summary: summary.to_owned(),
        report: None,
        at_ms: now,
    }
}

/// Orchestrators told about `worker`: its parent chat, and the chats whose
/// project worker destination contains the worker's item.
fn parents_of(state: &State, worker: &str) -> Vec<String> {
    let redirect = |candidate: &str| {
        let mut current = candidate;
        for _ in 0..state.handed_off_to.len() {
            match state.handed_off_to.get(current) {
                Some(next) => current = next,
                None => break,
            }
        }
        current.to_owned()
    };
    let live = |id: &str| state.chats.get(id).is_some_and(|chat| !chat.archived);
    if let Some(parent) = state.chats.get(worker).and_then(|chat| chat.parent.as_deref()) {
        let parent = redirect(parent);
        if parent != worker && live(&parent) {
            return if state.core.settings(&parent).enabled { vec![parent] } else { Vec::new() };
        }
    }
    state.destination_parents.values()
        .filter_map(|map| map.get(worker))
        .flatten()
        .map(|candidate| redirect(candidate))
        .filter(|candidate| candidate != worker && live(candidate) && state.core.settings(candidate).enabled)
        .max_by_key(|candidate| (state.chats[candidate].last_active, candidate.clone()))
        .into_iter().collect()
}

fn is_orchestrator(state: &State, chat_id: &str) -> bool {
    state.chats.get(chat_id).is_some_and(|chat| !chat.archived)
        && state.core.settings(chat_id).enabled
        && (state.chats.values().any(|chat| chat.parent.as_deref() == Some(chat_id))
            || state.destination_parents.values().any(|map| map.values().any(|candidates| candidates.iter().any(|candidate| candidate == chat_id))))
}

/// Worker chat → orchestrators, for one Space: a chat on item X is the
/// orchestrator of every chat on an item inside X's project worker
/// destination (`worker_destination_for`).
fn destination_parents(snapshot: &WorkOutlineSnapshot) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for item in &snapshot.items {
        if item.chat_ids.is_empty() {
            continue;
        }
        let Some(destination) = snapshot.worker_destination_for(&item.id) else {
            continue;
        };
        for worker_item in &snapshot.items {
            if worker_item.chat_ids.is_empty() || !snapshot.is_within(destination, &worker_item.id)
            {
                continue;
            }
            for worker in &worker_item.chat_ids {
                for orch in &item.chat_ids {
                    if orch != worker {
                        let list = out.entry(worker.clone()).or_default();
                        if !list.contains(orch) {
                            list.push(orch.clone());
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod hand_off_tests {
    use super::*;

    #[test]
    fn destination_selects_most_recent_enabled_live_coordinator() {
        let mut state = State::default();
        state.chats.insert("worker".into(), ChatInfo::default());
        state.chats.insert("first".into(), ChatInfo { last_active: 1, ..Default::default() });
        state.chats.insert("second".into(), ChatInfo { last_active: 2, ..Default::default() });
        state.destination_parents.entry("space".into()).or_default().insert("worker".into(), vec!["first".into(), "second".into()]);
        assert_eq!(parents_of(&state, "worker"), vec!["second"]);
        state.chats.get_mut("second").unwrap().archived = true;
        assert_eq!(parents_of(&state, "worker"), vec!["first"]);
        state.core.set_settings("first", WakeSettings { enabled: false, ..WakeSettings::default() });
        assert!(parents_of(&state, "worker").is_empty());
    }

    #[test]
    fn live_parent_precedes_destination_and_disabled_parent_does_not_fall_back() {
        let mut state = State::default();
        state.chats.insert("worker".into(), ChatInfo { parent: Some("parent".into()), ..Default::default() });
        state.chats.insert("parent".into(), ChatInfo::default());
        state.chats.insert("destination".into(), ChatInfo::default());
        state.destination_parents.entry("space".into()).or_default().insert("worker".into(), vec!["destination".into()]);
        assert_eq!(parents_of(&state, "worker"), vec!["parent"]);
        state.core.set_settings("parent", WakeSettings { enabled: false, ..WakeSettings::default() });
        assert!(parents_of(&state, "worker").is_empty());
        state.chats.get_mut("parent").unwrap().archived = true;
        assert_eq!(parents_of(&state, "worker"), vec!["destination"]);
    }

    #[test]
    fn hand_off_redirects_project_destination_wakes() {
        let mut state = State::default();
        state.chats.insert("worker".into(), ChatInfo { parent: Some("successor".into()), ..Default::default() });
        state.chats.insert("successor".into(), ChatInfo::default());
        state.destination_parents.entry("space".into()).or_default().insert("worker".into(), vec!["source".into()]);
        state.handed_off_to.insert("source".into(), "successor".into());
        assert_eq!(parents_of(&state, "worker"), vec!["successor"]);
    }
}

#[cfg(test)]
mod firm_link_tests {
    use super::firm_link;
    use mast_proto::ChatLinkSource;

    #[test]
    fn a_mentioned_link_neither_labels_nor_announces() {
        let id = "ENG-3219".to_string();
        assert_eq!(firm_link(Some(&id), Some(ChatLinkSource::Mentioned)), None);
        assert_eq!(firm_link(Some(&id), None), None);
        assert_eq!(
            firm_link(Some(&id), Some(ChatLinkSource::Manual)),
            Some(id.clone())
        );
        assert_eq!(firm_link(Some(&id), Some(ChatLinkSource::Branch)), Some(id));
    }
}
