//! Deterministic description of the small fixture world.
//!
//! This is a pure description: no host clock, random source, account or
//! filesystem. `materialize` turns it into repos and engine stores. One seed
//! drives every generated identifier and content choice; every timestamp is
//! relative to [`EPOCH_MS`].

use serde_json::{Value, json};

/// Fixed epoch for every timestamp in the world (same instant as the UI
/// snapshot fixture).
pub const EPOCH_MS: i64 = 1_788_900_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeState {
    Clean,
    /// Has an uncommitted change and an untracked file.
    Dirty,
    /// Created, then its directory removed (git still lists it as prunable).
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceFixture {
    pub id: String,
    pub name: String,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeFixture {
    pub space_id: String,
    pub repository: String,
    pub main: bool,
    pub path: String,
    pub branch: String,
    pub state: WorktreeState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatFixture {
    pub id: String,
    pub title: String,
    pub space_id: String,
    pub worktree: String,
    pub archived: bool,
    /// Transcript entries in `SessionMessageEntry` wire shape.
    pub entries: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemFixture {
    pub id: String,
    /// Operation id the engine derives `id` from.
    pub operation_id: String,
    pub space_id: String,
    pub title: String,
    pub parent: Option<usize>,
    /// `pending`, `active` or `completed`.
    pub status: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestFixture {
    pub item: usize,
    pub chat: usize,
    pub text: String,
    pub detail: String,
    pub answered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallWorld {
    pub preset: &'static str,
    pub shape: Option<crate::shape::WorldShape>,
    pub seed: u64,
    pub clock_ms: i64,
    pub spaces: Vec<SpaceFixture>,
    pub worktrees: Vec<WorktreeFixture>,
    pub chats: Vec<ChatFixture>,
    pub items: Vec<ItemFixture>,
    pub requests: Vec<RequestFixture>,
}

impl SmallWorld {
    pub const PRESET: &'static str = "small";

    /// Small preset at the fixed epoch.
    pub fn small(seed: u64) -> Self {
        Self::new(seed, EPOCH_MS)
    }

    /// Construct fixture identities and content from `seed` and `clock_ms`.
    pub fn new(seed: u64, clock_ms: i64) -> Self {
        let mut state = seed;
        let mut next = move || {
            // SplitMix64 makes the sequence independent of platform hashers.
            state = state.wrapping_add(0x9e3779b97f4a7c15);
            let mut value = state;
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
            value ^ (value >> 31)
        };
        let mut id = |kind: &str| format!("{kind}-{:016x}", next());
        let space_id = id("space");
        let spaces = vec![SpaceFixture {
            id: space_id.clone(),
            name: "Fixture project".into(),
            repository: "space-0".into(),
        }];
        let states = [
            WorktreeState::Clean,
            WorktreeState::Dirty,
            WorktreeState::Deleted,
        ];
        let worktrees: Vec<_> = states
            .iter()
            .enumerate()
            .map(|(index, state)| WorktreeFixture {
                space_id: space_id.clone(),
                repository: "space-0".into(),
                main: false,
                path: format!("worktree-{index}"),
                branch: format!("feature/fixture-{index}"),
                state: *state,
            })
            .collect();
        let titles = [
            "Review the workspace",
            "Design review",
            "Verify the changes",
            "Retry after failure",
            "Fresh chat",
        ];
        let chats: Vec<_> = titles
            .iter()
            .enumerate()
            .map(|(index, title)| ChatFixture {
                id: id("chat"),
                title: (*title).into(),
                space_id: space_id.clone(),
                worktree: worktrees[index % worktrees.len()].path.clone(),
                archived: false,
                entries: transcript(index, clock_ms),
            })
            .collect();
        // Outline: a root, Features and Bugs containers, nested items.
        // (title, parent index, status)
        let outline: [(&str, Option<usize>, &'static str); 12] = [
            ("Fixture project", None, "active"),
            ("Features", Some(0), "active"),
            ("Bugs", Some(0), "active"),
            ("Add search to the sidebar", Some(1), "active"),
            ("Keyboard navigation", Some(3), "active"),
            ("Search results grouping", Some(3), "pending"),
            ("Export transcript", Some(1), "completed"),
            ("Dark theme polish", Some(1), "pending"),
            ("Crash when a worktree is deleted", Some(2), "active"),
            ("Stale chat after restart", Some(2), "completed"),
            ("Scroll jumps on long diffs", Some(2), "active"),
            ("Tooltip clipped at window edge", Some(2), "pending"),
        ];
        let items: Vec<_> = outline
            .iter()
            .enumerate()
            .map(|(index, (title, parent, status))| {
                let operation_id = format!("{}-item-{index}", id("op"));
                ItemFixture {
                    id: mast_proto::derive_id(&operation_id, mast_proto::roles::ITEM),
                    operation_id,
                    space_id: space_id.clone(),
                    title: (*title).into(),
                    parent: *parent,
                    status,
                }
            })
            .collect();
        let requests = vec![
            RequestFixture {
                item: 3,
                chat: 0,
                text: "Approve the search plan?".into(),
                detail: "Two options: a sidebar filter or a command palette.".into(),
                answered: false,
            },
            RequestFixture {
                item: 8,
                chat: 2,
                text: "Which worktree should the fix land in?".into(),
                detail: "The original worktree was deleted.".into(),
                answered: true,
            },
        ];
        Self {
            preset: Self::PRESET,
            shape: None,
            seed,
            clock_ms,
            spaces,
            worktrees,
            chats,
            items,
            requests,
        }
    }

    /// Generate synthetic records for a counts-only shape.
    pub fn shaped(
        seed: u64,
        preset: &'static str,
        shape: &crate::shape::WorldShape,
    ) -> Result<Self, String> {
        if shape.spaces == 0
            || shape.repositories == 0
            || shape.chats < shape.archived_chats
            || shape.worktrees_per_repository_median > shape.worktrees_per_repository_max
            || shape.chats_per_space_median > shape.chats_per_space_max
        {
            return Err("inconsistent world shape".into());
        }
        let spaces: Vec<_> = (0..shape.spaces)
            .map(|i| SpaceFixture {
                id: format!("space-{seed:016x}-{i}"),
                name: format!("Fixture Space {i}"),
                repository: format!("repository-{}", i % shape.repositories),
            })
            .collect();
        let mut worktrees = Vec::new();
        for repo in 0..shape.repositories {
            let count = if repo == 0 {
                shape.worktrees_per_repository_max
            } else {
                shape.worktrees_per_repository_median
            };
            for index in 0..count {
                worktrees.push(WorktreeFixture {
                    space_id: spaces[repo % spaces.len()].id.clone(),
                    repository: format!("repository-{repo}"),
                    main: index == 0,
                    path: if index == 0 {
                        format!("../repository-{repo}")
                    } else {
                        format!("repo-{repo}-worktree-{index}")
                    },
                    branch: format!("feature/fixture-{repo}-{index}"),
                    state: WorktreeState::Clean,
                });
            }
        }
        if worktrees.is_empty() && shape.chats > 0 {
            return Err("world shape needs worktrees for chats".into());
        }
        let counts = distribute(
            shape.spaces,
            shape.chats,
            shape.chats_per_space_median,
            shape.chats_per_space_max,
        )?;
        let mut chats = Vec::new();
        for (space, count) in spaces.iter().zip(counts) {
            let repo = &space.repository;
            let trees: Vec<_> = worktrees.iter().filter(|w| &w.repository == repo).collect();
            if count > 0 && trees.is_empty() {
                return Err("Space with chats has no repository worktree".into());
            }
            for index in 0..count {
                let i = chats.len();
                let messages = if i % 10 == 0 {
                    shape.messages_per_chat_p90
                } else {
                    shape.messages_per_chat_median
                };
                chats.push(ChatFixture {
                    id: format!("chat-{seed:016x}-{i}"),
                    title: format!("Fixture chat {i}"),
                    space_id: space.id.clone(),
                    worktree: trees[index % trees.len()].path.clone(),
                    archived: i >= shape.chats - shape.archived_chats,
                    entries: (0..messages)
                        .map(|n| {
                            user(
                                &format!("message-{n}"),
                                "Fixture message.",
                                EPOCH_MS + n as i64,
                            )
                        })
                        .collect(),
                });
            }
        }
        let statuses = [
            ("pending", shape.pending_items),
            ("active", shape.active_items),
            ("completed", shape.completed_items),
        ];
        let mut items = Vec::new();
        for (status, count) in statuses {
            for _ in 0..count {
                let index = items.len();
                let space_index = index % spaces.len();
                let operation_id = format!("fixture-{seed:016x}-item-{index}");
                items.push(ItemFixture {
                    id: mast_proto::derive_id(&operation_id, mast_proto::roles::ITEM),
                    operation_id,
                    space_id: spaces[space_index].id.clone(),
                    title: format!("Fixture item {index}"),
                    parent: if shape.outline_depth > 1 && index >= spaces.len() {
                        Some(if index / spaces.len() < shape.outline_depth {
                            index - spaces.len()
                        } else {
                            space_index
                        })
                    } else {
                        None
                    },
                    status,
                });
            }
        }
        let mut requests = Vec::new();
        let count = shape.open_requests + shape.resolved_requests;
        if count > 0 && (items.is_empty() || chats.is_empty()) {
            return Err("requests need items and chats".into());
        }
        let mut first_chat = std::collections::HashMap::new();
        for (index, chat) in chats.iter().enumerate() {
            first_chat.entry(chat.space_id.as_str()).or_insert(index);
        }
        let eligible: Vec<_> = items.iter().enumerate().filter_map(|(index, item)|
            first_chat.get(item.space_id.as_str()).map(|chat| (index, *chat))).collect();
        if count > 0 && eligible.is_empty() {
            return Err("requests need items in Spaces with chats".into());
        }
        for index in 0..count {
            let (item, chat) = eligible[index % eligible.len()];
            requests.push(RequestFixture {
                item,
                chat,
                text: format!("Fixture request {index}?"),
                detail: "Fixture detail.".into(),
                answered: index >= shape.open_requests,
            });
        }
        Ok(Self {
            preset,
            shape: Some(shape.clone()),
            seed,
            clock_ms: EPOCH_MS,
            spaces,
            worktrees,
            chats,
            items,
            requests,
        })
    }
}

/// Build a sorted distribution with the requested total, upper median and maximum.
fn distribute(count: usize, total: usize, median: usize, max: usize) -> Result<Vec<usize>, String> {
    if count == 0 || median > max {
        return Err("invalid distribution".into());
    }
    let middle = count / 2;
    let mut values = vec![0; count];
    for value in &mut values[middle..] {
        *value = median;
    }
    values[count - 1] = max;
    let mut remaining = total
        .checked_sub(values.iter().sum())
        .ok_or("distribution total too small")?;
    for i in (0..middle).rev().chain((middle..count - 1).rev()) {
        let limit = if i < middle { median } else { max };
        let increment = remaining.min(limit - values[i]);
        values[i] += increment;
        remaining -= increment;
    }
    if remaining != 0 {
        return Err("distribution total too large".into());
    }
    Ok(values)
}

fn user(id: &str, text: &str, at: i64) -> Value {
    json!({"id":id,"role":"user","parts":[{"id":"t0","kind":"text","text":text}],"createdAt":at,"deviceId":"local"})
}

fn assistant(id: &str, parts: Vec<Value>, at: i64) -> Value {
    json!({"id":id,"role":"assistant","parts":parts,"createdAt":at,"deviceId":"local","status":"complete"})
}

fn text(id: &str, text: &str) -> Value {
    json!({"id":id,"kind":"text","text":text})
}

fn tool(id: &str, call: Value, output: &str) -> Value {
    json!({"id":id,"kind":"tool","call":call,"resolved":true,"output":output})
}

/// Transcript for chat `index`, with timestamps relative to `clock_ms`.
fn transcript(index: usize, clock_ms: i64) -> Vec<Value> {
    let at = clock_ms + index as i64 * 600_000;
    let long = "The layout is built from three regions. The sidebar owns navigation and never \
                scrolls with the transcript. The transcript keeps a comfortable reading width \
                even when the window becomes narrow, and long lines wrap inside it.\n\n\
                - The first list item has enough text to wrap onto another line while its \
                continuation stays aligned with the start of the item.\n\
                - The second item shares the same indentation.\n\n\
                ```rust\nfn validate_layout() {\n    assert!(controls_align());\n}\n```";
    match index {
        0 => vec![
            user("user-0", "Review the workspace layout.", at),
            assistant(
                "assistant-0",
                vec![
                    text("a0", long),
                    tool(
                        "tool-0",
                        json!({"kind":"exec","command":"cargo test"}),
                        "test result: ok. 41 passed",
                    ),
                    tool(
                        "tool-1",
                        json!({"kind":"editFile","path":"src/layout.rs","old_string":"width: 400","new_string":"width: 480"}),
                        "Edited src/layout.rs",
                    ),
                    text(
                        "a1",
                        "I widened the transcript column and re-ran the tests.",
                    ),
                ],
                at + 1_000,
            ),
        ],
        1 => vec![
            user("user-0", "Walk me through the design.", at),
            assistant("assistant-0", vec![text("a0", long)], at + 1_000),
            user("user-1", "Can the sidebar collapse?", at + 2_000),
            assistant(
                "assistant-1",
                vec![text("a0", "Yes. It collapses to the icon rail.")],
                at + 3_000,
            ),
        ],
        2 => vec![
            user("user-0", "Verify the changes.", at),
            assistant(
                "assistant-0",
                vec![
                    tool(
                        "tool-0",
                        json!({"kind":"exec","command":"git diff --stat"}),
                        " src/layout.rs | 2 +-",
                    ),
                    tool(
                        "tool-1",
                        json!({"kind":"exec","command":"cargo check"}),
                        "error[E0425]: cannot find value `width`",
                    ),
                    text(
                        "a0",
                        "The check failed on an unresolved name; I fixed it and re-ran.",
                    ),
                ],
                at + 1_000,
            ),
        ],
        3 => vec![
            user("user-0", "Review the sidebar spacing.", at),
            assistant(
                "assistant-0",
                vec![
                    json!({"id":"error","kind":"error","message":"The service is overloaded. Please try again."}),
                ],
                at + 1_000,
            ),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_and_clock_determine_the_world_description() {
        let first = SmallWorld::new(42, 946_684_800_000);
        assert_eq!(first, SmallWorld::new(42, 946_684_800_000));
        assert_ne!(first, SmallWorld::new(43, 946_684_800_000));
        assert_eq!(
            (
                first.spaces.len(),
                first.worktrees.len(),
                first.chats.len(),
                first.items.len(),
                first.requests.len()
            ),
            (1, 3, 5, 12, 2)
        );
        for request in &first.requests {
            assert!(request.item < first.items.len() && request.chat < first.chats.len());
        }
        assert_eq!(first.requests.iter().filter(|r| r.answered).count(), 1);
        for (index, item) in first.items.iter().enumerate() {
            assert!(item.parent.is_none_or(|parent| parent < index));
        }
    }

    #[test]
    fn medium_world_matches_shape_and_seed() {
        let shape = crate::shape::WorldShape::medium();
        let world = SmallWorld::shaped(7, "medium", &shape).unwrap();
        assert_eq!(world, SmallWorld::shaped(7, "medium", &shape).unwrap());
        assert_ne!(world, SmallWorld::shaped(8, "medium", &shape).unwrap());
        assert_eq!(world.spaces.len(), shape.spaces);
        assert_eq!(world.chats.len(), shape.chats);
        assert_eq!(
            world.chats.iter().filter(|chat| chat.archived).count(),
            shape.archived_chats
        );
        assert_eq!(
            world.items.len(),
            shape.pending_items + shape.active_items + shape.completed_items
        );
        assert_eq!(
            world
                .requests
                .iter()
                .filter(|request| !request.answered)
                .count(),
            shape.open_requests
        );
        let mut counts: Vec<_> = world
            .spaces
            .iter()
            .map(|space| {
                world
                    .chats
                    .iter()
                    .filter(|chat| chat.space_id == space.id)
                    .count()
            })
            .collect();
        counts.sort_unstable();
        assert_eq!(counts[counts.len() / 2], shape.chats_per_space_median);
        assert_eq!(*counts.last().unwrap(), shape.chats_per_space_max);
    }

    #[test]
    fn requests_use_spaces_with_chats() {
        let mut shape = crate::shape::WorldShape::medium();
        shape.spaces = 3;
        shape.repositories = 1;
        shape.chats = 2;
        shape.archived_chats = 0;
        shape.chats_per_space_median = 0;
        shape.chats_per_space_max = 2;
        shape.pending_items = 3;
        shape.active_items = 0;
        shape.completed_items = 0;
        let world = SmallWorld::shaped(1, "scale", &shape).unwrap();
        assert!(world.requests.iter().all(|request| world.items[request.item].space_id == world.chats[request.chat].space_id));
    }
}
