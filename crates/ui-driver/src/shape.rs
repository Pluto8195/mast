//! Counts describing a driver world without identifying any source records.

use serde::{Deserialize, Serialize};

/// Aggregate dimensions of a world. All values are nonnegative counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldShape {
    /// Number of Spaces.
    pub spaces: usize,
    /// Number of distinct repositories.
    pub repositories: usize,
    /// Maximum number of worktrees associated with one repository.
    pub worktrees_per_repository_max: usize,
    /// Median number of worktrees associated with a repository.
    pub worktrees_per_repository_median: usize,
    /// Maximum number of chats in one Space.
    pub chats_per_space_max: usize,
    /// Median number of chats per Space.
    pub chats_per_space_median: usize,
    /// Total number of chats, including archived chats.
    pub chats: usize,
    /// Number of archived chats.
    pub archived_chats: usize,
    /// Number of pending work items.
    pub pending_items: usize,
    /// Number of active work items.
    pub active_items: usize,
    /// Number of completed work items.
    pub completed_items: usize,
    /// Maximum number of item levels in an outline, including its root.
    pub outline_depth: usize,
    /// Number of pull request links attached to chats.
    pub chat_pr_links: usize,
    /// Number of pull request references attached to items.
    pub item_pr_refs: usize,
    /// Number of open Required Actions.
    pub open_requests: usize,
    /// Number of resolved Required Actions.
    pub resolved_requests: usize,
    /// Number of friction reports.
    pub friction_reports: usize,
    /// Median number of transcript messages per chat.
    pub messages_per_chat_median: usize,
    /// Ninetieth percentile number of transcript messages per chat.
    pub messages_per_chat_p90: usize,
    /// Median number of branches per repository.
    pub branches_per_repository_median: usize,
    /// Maximum number of branches in one repository.
    pub branches_per_repository_max: usize,
}

impl WorldShape {
    /// Fixed, synthetic dimensions for the medium preset.
    pub fn medium() -> Self {
        Self {
            spaces: 20,
            repositories: 3,
            worktrees_per_repository_max: 12,
            worktrees_per_repository_median: 3,
            chats_per_space_max: 20,
            chats_per_space_median: 15,
            chats: 300,
            archived_chats: 30,
            pending_items: 150,
            active_items: 150,
            completed_items: 100,
            outline_depth: 4,
            chat_pr_links: 300,
            item_pr_refs: 60,
            open_requests: 30,
            resolved_requests: 20,
            friction_reports: 10,
            messages_per_chat_median: 4,
            messages_per_chat_p90: 12,
            branches_per_repository_median: 8,
            branches_per_repository_max: 16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorldShape;

    #[test]
    fn shape_round_trip() {
        let json = r#"{"spaces":2,"repositories":1,"worktrees_per_repository_max":3,"worktrees_per_repository_median":3,"chats_per_space_max":4,"chats_per_space_median":2,"chats":6,"archived_chats":1,"pending_items":2,"active_items":3,"completed_items":1,"outline_depth":3,"chat_pr_links":4,"item_pr_refs":2,"open_requests":1,"resolved_requests":1,"friction_reports":1,"messages_per_chat_median":2,"messages_per_chat_p90":5,"branches_per_repository_median":4,"branches_per_repository_max":4}"#;
        let shape: WorldShape = serde_json::from_str(json).unwrap();
        assert_eq!(
            shape,
            serde_json::from_str(&serde_json::to_string(&shape).unwrap()).unwrap()
        );
    }

    #[test]
    fn medium_dimensions() {
        let shape = WorldShape::medium();
        assert_eq!(shape.spaces, 20);
        assert_eq!(shape.repositories, 3);
        assert_eq!(shape.worktrees_per_repository_max, 12);
        assert_eq!(shape.chats, 300);
        assert_eq!(
            shape.pending_items + shape.active_items + shape.completed_items,
            400
        );
        assert_eq!(shape.chat_pr_links, 300);
        assert_eq!(shape.open_requests, 30);
    }
}
