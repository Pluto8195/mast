//! The side panel's Team tab: one row per worker of an orchestrating chat,
//! a roll-up line on top, and the Item tab's compact "top workers" block. Both
//! read one [`TeamFacts`] sheet (`crate::team`), so they cannot disagree.
//!
//! ⌥⌘T opens the tab from anywhere; a plain `t` does the same while the side
//! panel itself (not a text field inside it) holds focus. Both do nothing for
//! a chat without workers.
use super::*;
use crate::components::shell_parts::team_button;

use crate::shell::chat_item_ui::{ChipTone, chip_palette};
use crate::team::{
    ITEM_TAB_WORKERS, SUGGEST_ARCHIVE_AT, TeamFacts, TeamInput, WorkerFacts, WorkerPr,
    team,
};

impl Shell {
    #[cfg(feature = "snapshot")]
    pub(crate) fn fixture_snapshot_team(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            state.chats.push(serde_json::from_value(serde_json::json!({
                "id": "fixture-worker", "deviceId": "local", "spaceId": "fixture-space",
                "parentChatId": "fixture-chat", "title": "Validate the release",
                "archived": false, "createdAt": "2026-09-08T00:00:00Z"
            })).unwrap());
            // Keep the in-memory send active throughout the multi-cell render.
            state.begin_pending_send("fixture-worker", "fixture-message", chrono::Utc::now() + chrono::Duration::hours(1));
        });
        self.active_chat = "fixture-chat".into();
        let key = self.panel_key(cx);
        self.panels.update(&key, |panel| {
            panel.changes_open = true;
            panel.right_active = RightSurface::Team;
        });
    }
    /// The selected chat's workers. Link statuses (PR checks, diff stats) come
    /// from the header-link cache; missing ones are requested and the tab
    /// repaints when they land.
    pub(super) fn team_facts(&mut self, chat_id: &str, cx: &mut Context<Self>) -> TeamFacts {
        let facts = {
            let state = self.state.read(cx);
            let Some(chat) = state.chats.iter().find(|chat| chat.id == chat_id) else {
                return TeamFacts::default();
            };
            let now = chrono::Utc::now();
            let indicators = state
                .chats
                .iter()
                .map(|chat| (chat.id.clone(), state.display_status_for(chat, now)))
                .collect();
            let snapshot = chat
                .space_id
                .as_deref()
                .and_then(|id| state.work_outline(id).snapshot().cloned());
            let links = self
                .chat_header_links
                .iter()
                .filter_map(|(id, entry)| Some((id.clone(), entry.status.clone()?)))
                .collect();
            team(&TeamInput {
                chat_id,
                chats: &state.chats,
                indicators: &indicators,
                snapshot: snapshot.as_deref(),
                links: &links,
                now,
            })
        };
        for worker in &facts.workers {
            self.ensure_chat_header_links(worker.chat_id.clone(), cx);
        }
        facts
    }

    /// ⌥⌘T / `t`: open the Team tab; nothing happens without workers.
    pub(crate) fn open_team_tab(&mut self, cx: &mut Context<Self>) {
        if self.active_chat.is_empty() {
            return;
        }
        let chat_id = self.active_chat.clone();
        if !self
            .settings
            .team_tab_visibility
            .visible(!self.team_facts(&chat_id, cx).is_empty())
        {
            return;
        }
        self.leave_overview_for_chat(cx);
        self.set_surfaces_open(true, cx);
        self.set_right_active(RightSurface::Team, cx);
        cx.notify();
    }

    /// The focus target of the Item and Team tabs' bodies.
    pub(super) fn team_panel_focus(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        self.team_panel_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// The sticky chip after "Item": present only while the chat has workers.
    pub(super) fn render_team_tab_chip(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chat_id = self.active_chat.clone();
        let team = self.team_facts(&chat_id, cx);
        if !self.settings.team_tab_visibility.visible(!team.is_empty()) {
            return None;
        }
        let theme = Theme::of(cx).clone();
        let active = self.resolved_right_active(cx) == RightSurface::Team;
        let waiting = team.waiting() > 0;
        Some(
            crate::components::shell_parts::panel_tab(&theme, "right-surface-team-tab", "Team", format!("Team {}", team.live().count()), active)
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_right_active(RightSurface::Team, cx);
                }))
                .when(waiting, |el| {
                    el.child(
                        div()
                            .debug_selector(|| "right-surface-team-dot".into())
                            .child(crate::status_style::indicator_for_theme(crate::status_style::Status::Waiting, &theme)),
                    )
                })
                .into_any_element(),
        )
    }

    fn render_team_summary(team: &TeamFacts, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let parts = team.roll_up_parts();
        if parts.is_empty() { return div().child("No active workers").into_any_element(); }
        let mut summary = crate::components::linear_parts::team_summary(theme);
        for (index, (status, label)) in parts.into_iter().enumerate() {
            if index > 0 { summary = summary.child("·"); }
            let indicator = crate::status_style::animated_indicator(status, theme.appearance, format!("team-summary-{index}"), cx.entity_id(), cx);
            summary = summary.child(indicator).child(SharedString::from(label));
        }
        summary.into_any_element()
    }

    /// The Team tab's body: roll-up line, then one row per worker.
    pub(super) fn render_team_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let chat_id = self.active_chat.clone();
        let team = self.team_facts(&chat_id, cx);
        let focus = self.team_panel_focus(cx);
        let mut col = crate::components::linear_parts::team_column(&theme, false);
        if self
            .state
            .read(cx)
            .chats
            .iter()
            .any(|chat| chat.id == chat_id && chat.archived)
        {
            col = col.child(
                crate::components::shell_parts::team_notice(&theme, "team-coordinator-archived", "Coordinator archived — hand off or unarchive"),
            );
        }
        if team.is_empty() {
            col = col.child(
                crate::components::shell_parts::team_notice(&theme, "team-empty", "This chat has no workers."),
            );
        } else {
            col = col.child(
                crate::components::shell_parts::team_rollup(&theme)
                    .child(Self::render_team_summary(&team, &theme, cx)),
            );
            col = col.child(self.render_team_wake_header(&chat_id, &theme, cx));
            col = col.child(self.render_team_archive_block(&chat_id, &team, &theme, cx));
            for worker in team.live() {
                col = col.child(self.render_worker_row(worker, false, &theme, cx));
            }
            col = col.children(self.render_team_archived(&chat_id, &team, &theme, cx));
        }
        self.panel_root("team-tab", focus, col, cx)
    }

    /// The scrolling, focusable root shared by the Item and Team tabs.
    pub(super) fn panel_root(
        &mut self,
        id: &'static str,
        focus: FocusHandle,
        body: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tracked = focus.clone();
        let theme = Theme::of(cx).clone();
        crate::components::shell_parts::panel_root(&theme, id)
            .track_focus(&focus)
            .key_context("TeamPanel")
            .on_action(
                cx.listener(move |this, _: &OpenTeamTabFromPanel, window, cx| {
                    if tracked.is_focused(window)
                        && this.resolved_right_active(cx) != RightSurface::Team
                    {
                        this.open_team_tab(cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .child(body)
            .into_any_element()
    }

    /// The Item tab's block: the roll-up line, the first workers, and a link
    /// to the Team tab.
    pub(super) fn render_item_tab_team(
        &mut self,
        team: &TeamFacts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut block = crate::components::linear_parts::team_column(theme, true)
            .debug_selector(|| "chat-item-team".into())
            .child(
                crate::components::shell_parts::item_team_rollup(theme)
                    .child(Self::render_team_summary(team, theme, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.open_team_tab(cx))),
            );
        for worker in team.live().take(ITEM_TAB_WORKERS) {
            block = block.child(self.render_worker_row(worker, true, theme, cx));
        }
        if team.live().count() > ITEM_TAB_WORKERS {
            block = block.child(
                crate::components::shell_parts::item_team_all(theme, team.live().count())
                    .on_click(cx.listener(|this, _, _, cx| this.open_team_tab(cx))),
            );
        }
        block.into_any_element()
    }

    fn chip(theme: &Theme, id: String, label: String, tone: ChipTone) -> gpui::Stateful<gpui::Div> {
        let (border, text) = chip_palette(theme, tone);
        crate::components::shell_parts::panel_chip(theme, id, label, border, text)
    }

    fn pr_tone(pr: &WorkerPr) -> ChipTone {
        use mast_engine::pr_ticket_cache::ChecksStatus;
        match (pr.checks, pr.review) {
            (Some(ChecksStatus::Failing), _) | (_, Some("changes requested")) => ChipTone::Warn,
            _ => ChipTone::Link,
        }
    }

    /// One worker. `compact` (the Item tab) is title, state and age plus the
    /// activity line; the Team tab adds chips, diff stats and the
    /// Ready-to-merge jump.
    fn render_worker_row(
        &mut self,
        worker: &WorkerFacts,
        compact: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let prefix = if compact {
            "chat-item-worker"
        } else {
            "team-worker"
        };
        let id = worker.chat_id.clone();
        let selector = format!("{prefix}-{id}");
        let now = chrono::Utc::now();
        let age = worker
            .last_active
            .map(|at| mast_proto::view::format_time_ago(at, now))
            .unwrap_or_default();
        let status = crate::status_style::status_style(worker.state.into(), theme.appearance);
        let indicator = crate::components::shell_parts::activity_indicator(worker.state.into(), format!("{prefix}-status-{id}"), theme, cx.entity_id(), cx);
        let open_id = id.clone();
        let changes_id = id.clone();
        let changes_branch = worker.branch.clone();
        let changes_selector = format!("{prefix}-changes-{id}");
        let menu_id = id.clone();
        let unseen = self.unseen_for_worker(&id);
        let stalled = self.state.read(cx).session_for(&id).and_then(|session| {
            if session.status == mast_proto::SessionStatus::AwaitingInput {
                Some(("waiting for input".to_owned(), crate::status_style::status_style(crate::status_style::Status::Waiting, theme.appearance).color))
            } else {
                session
                    .last_turn_tool_failures
                    .as_ref()
                    .map(|failure| (format!("{} tool calls failed", failure.count), theme.danger))
            }
        });
        let mut row = crate::components::shell_parts::worker_row(theme, selector)
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                let closeable = crate::chat_closeout::is_closeout_candidate(this.state.read(cx).chats.iter()
                    .find(|chat| chat.id == menu_id).and_then(|chat| chat.cwd.as_deref()));
                this.chat_menu.open(ChatMenuState { chat_id: menu_id.clone(), position: event.position, page: ChatMenuPage::Root, closeable });
                this.load_branch_review_target(menu_id.clone(), cx);
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.open_chat(open_id.clone(), cx)))
            .child(
                crate::components::linear_parts::team_worker_header(theme)
                    .child(div().flex_none().child(indicator))
                    .child(
                        crate::components::shell_parts::worker_title(theme, worker.title.clone()),
                    )
                    .children(stalled.map(|(label, color)| {
                        crate::components::shell_parts::worker_metadata(theme, label, color)
                            .debug_selector({
                                let selector = format!("{prefix}-stalled-{id}");
                                move || selector.clone()
                            })
                    }))
                    .children((!unseen.is_empty()).then(|| {
                        let tip = SharedString::from(format!(
                            "Unseen by the orchestrator:\n{}",
                            unseen.join("\n")
                        ));
                        crate::components::shell_parts::worker_unseen_badge(theme, format!("{prefix}-unseen-{id}"))
                            .tooltip(move |_, cx| {
                                cx.new(|_| SurfaceTabTooltip { text: tip.clone() }).into()
                            })
                    }))
                    .child(
                        crate::components::shell_parts::worker_metadata(theme, worker.state.label().to_owned(), status.color),
                    )
                    .child(
                        crate::components::shell_parts::worker_metadata(theme, age, theme.text_faint),
                    )
                    .child(
                        crate::components::shell_parts::worker_action_link(theme, crate::icons::FILE_CODE, "Changes")
                            .id(SharedString::from(changes_selector.clone()))
                            .debug_selector(move || changes_selector.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_request_diff(changes_id.clone(), changes_branch.clone(), cx);
                            })),
                    ),
            );
        if let Some(report) = self.latest_report_for_worker(&id) {
            // The worker's own report is its activity line; click for the full text.
            let expanded = self.wakes.expanded_reports.contains(&id);
            let line = crate::team::activity_line(&report).unwrap_or_default();
            let toggle_id = id.clone();
            row = row.child(
                crate::components::shell_parts::worker_report_toggle(theme, format!("{prefix}-report-toggle-{id}"), format!("{prefix}-activity-{id}"), format!(
                        "{} Report: {line}",
                        if expanded { "\u{25be}" } else { "\u{25b8}" }
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_report_expanded(&toggle_id, cx);
                    })),
            );
            if expanded {
                row = row.child(
                    crate::components::shell_parts::worker_activity(theme, format!("{prefix}-report-{id}"), report, true),
                );
            }
        } else if let Some(activity) = &worker.activity {
            row = row.child(
                crate::components::shell_parts::worker_activity(theme, format!("{prefix}-activity-{id}"), activity.clone(), false),
            );
        }
        if !compact {
            let mut chips = crate::components::linear_parts::team_worker_metadata(theme, true);
            let mut any = false;
            if let Some(branch) = &worker.branch {
                any = true;
                chips = chips.child(Self::chip(
                    theme,
                    format!("team-branch-{id}"),
                    format!("\u{2387} {branch}"),
                    ChipTone::Dim,
                ));
            }
            for (ix, pr) in worker.prs.iter().enumerate() {
                any = true;
                let url = pr.url.clone();
                chips = chips.child(
                    Self::chip(
                        theme,
                        format!("team-pr-{id}-{ix}"),
                        pr.label(),
                        Self::pr_tone(pr),
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if !url.is_empty() {
                            let external = window.modifiers().secondary();
                            this.open_web_reference(&url, external, window, cx)
                        }
                    })),
                );
            }
            if let Some(ticket) = &worker.ticket {
                any = true;
                let url = ticket.url.clone();
                chips = chips.child(
                    Self::chip(
                        theme,
                        format!("team-ticket-{id}"),
                        ticket.identifier.clone(),
                        ChipTone::Link,
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if let Some(url) = &url {
                            let external = window.modifiers().secondary();
                            this.open_web_reference(url, external, window, cx)
                        }
                    })),
                );
            }
            if let Some((added, removed)) = worker.diff {
                any = true;
                chips = chips.child(
                    crate::components::shell_parts::worker_diff(theme, format!("team-diff-{id}"), added, removed),
                );
            }
            if any {
                row = row.child(chips);
            }
        }
        if !compact && let Some(verdict) = worker.archivable.as_ref().filter(|a| a.archivable) {
            row = row.child(
                crate::components::shell_parts::worker_archive_hint(theme, format!("team-archivable-{id}"), &verdict.reason),
            );
        }
        if let Some(ready) = &worker.ready {
            let text = ready.text.clone();
            let mut references = {
                let state = self.state.read(cx);
                state
                    .work_outline(&ready.space_id)
                    .snapshot()
                    .and_then(|snapshot| snapshot.user_action_request(&ready.request_id))
                    .map(|request| {
                        request
                            .links
                            .iter()
                            .filter_map(|link| match &link.target {
                                mast_proto::work_outline_user_actions::RequestLinkTarget::Pr {
                                    url,
                                }
                                | mast_proto::work_outline_user_actions::RequestLinkTarget::Url {
                                    url,
                                } => crate::work_outline::required_actions::parse_handoff_pr(url),
                                _ => None,
                            })
                            .chain(
                                request
                                    .facts
                                    .iter()
                                    .filter(|fact| {
                                        fact.label.eq_ignore_ascii_case("pr")
                                            || fact.label.eq_ignore_ascii_case("pull request")
                                    })
                                    .filter_map(|fact| {
                                        crate::work_outline::required_actions::parse_handoff_pr(
                                            &fact.value,
                                        )
                                    }),
                            )
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            if worker.branch.is_some() {
                references.extend(worker.prs.iter().filter_map(|pr| {
                    crate::work_outline::required_actions::parse_handoff_pr(&pr.url)
                }));
            }
            for pr in references {
                crate::github::store::request_status(&self.state, &pr, false, cx);
            }
            let lifecycle = {
                let state = self.state.read(cx);
                state
                    .work_outline(&ready.space_id)
                    .snapshot()
                    .and_then(|snapshot| snapshot.user_action_request(&ready.request_id))
                    .and_then(|request| {
                        let explicit_pr = request.links.iter().any(|link| match &link.target {
                            mast_proto::work_outline_user_actions::RequestLinkTarget::Pr {
                                url,
                            }
                            | mast_proto::work_outline_user_actions::RequestLinkTarget::Url {
                                url,
                            } => crate::work_outline::required_actions::parse_handoff_pr(url)
                                .is_some(),
                            _ => false,
                        }) || request.facts.iter().any(|fact| {
                            (fact.label.eq_ignore_ascii_case("pr")
                                || fact.label.eq_ignore_ascii_case("pull request"))
                                && crate::work_outline::required_actions::parse_handoff_pr(
                                    &fact.value,
                                )
                                .is_some()
                        });
                        let worker_prs: std::collections::HashSet<_> = worker
                            .prs
                            .iter()
                            .filter_map(|pr| {
                                crate::work_outline::required_actions::parse_handoff_pr(&pr.url)
                            })
                            .map(|pr| pr.key())
                            .collect();
                        crate::work_outline::required_actions::handoff_lifecycle_parts(
                            request.effective_kind(),
                            &request.links,
                            &request.facts,
                            state.github.ready_statuses().filter(|status| {
                                explicit_pr || worker_prs.contains(&status.pr_ref.key())
                            }),
                        )
                    })
            };
            let ready_color = crate::status_style::status_style(crate::status_style::Status::Ready, theme.appearance).color;
            row = row.child(
                crate::components::linear_parts::team_worker_metadata(theme, false)
                    .child(
                        crate::components::shell_parts::worker_ready_label(text, ready_color),
                    )
                    .when_some(lifecycle, |el, lifecycle| {
                        el.child(Self::chip(
                            theme,
                            format!("team-handoff-lifecycle-{id}"),
                            lifecycle.to_owned(),
                            ChipTone::Dim,
                        ))
                    })
                    .child(
                        crate::components::shell_parts::worker_jump_button(theme, format!("{prefix}-jump-{id}"), ready_color)
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_item_tab(cx);
                            })),
                    ),
            );
        }
        row.into_any_element()
    }
}

/// Small text button used by the Team tab's archive controls.
impl Shell {
    /// "Archive finished (N)", the N >= 3 suggestion, and the confirmation.
    fn render_team_archive_block(
        &mut self,
        chat_id: &str,
        team: &TeamFacts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let finished = team.finished();
        let n = finished.len();
        let block = crate::components::linear_parts::team_archive_block(theme);
        if n == 0 {
            self.wakes.archive_confirm.remove(chat_id);
            return block.into_any_element();
        }
        if self.wakes.archive_confirm.contains(chat_id) {
            let mut confirm = crate::components::shell_parts::archive_confirmation(theme, n);
            for worker in &finished {
                let selector = format!("team-archive-confirm-row-{}", worker.chat_id);
                let reason = worker
                    .archivable
                    .as_ref()
                    .map(|a| a.reason.clone())
                    .unwrap_or_default();
                confirm = confirm.child(
                    crate::components::shell_parts::archive_confirmation_worker(theme, selector, &worker.title, &reason),
                );
            }
            let go_chat = chat_id.to_owned();
            let cancel_chat = chat_id.to_owned();
            confirm = confirm
                .child(
                    crate::components::shell_parts::archive_confirmation_note(theme),
                )
                .child(
                    crate::components::linear_parts::team_archive_actions(theme, true)
                        .child(
                            team_button(
                                theme,
                                "team-archive-confirm-go",
                                format!("Archive {n}"),
                                true,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.archive_finished_workers(&go_chat, cx),
                            )),
                        )
                        .child(
                            team_button(
                                theme,
                                "team-archive-confirm-cancel",
                                "Cancel".into(),
                                false,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.wakes.archive_confirm.remove(&cancel_chat);
                                    cx.notify();
                                },
                            )),
                        ),
                );
            return block.child(confirm).into_any_element();
        }
        let ask_chat = chat_id.to_owned();
        let mut line = crate::components::linear_parts::team_archive_actions(theme, false);
        if n >= SUGGEST_ARCHIVE_AT {
            line = line.child(
                crate::components::shell_parts::archive_suggestion(theme, n),
            );
        } else {
            line = line.child(div().flex_1());
        }
        line = line.child(
            team_button(
                theme,
                "team-archive-finished",
                format!("Archive finished ({n})"),
                true,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.wakes.archive_confirm.insert(ask_chat.clone());
                cx.notify();
            })),
        );
        block.child(line).into_any_element()
    }

    /// "Show archived (N)" and, when on, the archived workers with Unarchive.
    fn render_team_archived(
        &mut self,
        chat_id: &str,
        team: &TeamFacts,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let count = team.archived().count();
        if count == 0 {
            return Vec::new();
        }
        let shown = self.wakes.show_archived.contains(chat_id);
        let toggle_chat = chat_id.to_owned();
        let mut out = vec![
            crate::components::shell_parts::archived_worker_toggle(theme, shown, count)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.wakes.show_archived.remove(&toggle_chat) {
                        this.wakes.show_archived.insert(toggle_chat.clone());
                    }
                    cx.notify();
                }))
                .into_any_element(),
        ];
        if shown {
            for worker in team.archived() {
                let id = worker.chat_id.clone();
                let selector = format!("team-archived-{id}");
                let restore_id = id.clone();
                let button_id = format!("team-unarchive-{id}");
                out.push(
                    crate::components::shell_parts::archived_worker_row(theme, selector)
                        .child(
                            crate::components::shell_parts::archived_worker_title(theme, worker.title.clone()),
                        )
                        .child(
                            team_button(theme, button_id, "Unarchive".into(), false).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.set_worker_archived(&restore_id, false, cx)
                                }),
                            ),
                        )
                        .into_any_element(),
                );
            }
        }
        out
    }

    /// Confirmed "Archive finished": archive every worker the rule rates
    /// finished right now (re-evaluated, so a chat that started working since
    /// the dialog opened is skipped).
    fn archive_finished_workers(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.wakes.archive_confirm.remove(chat_id);
        let ids: Vec<String> = self
            .team_facts(chat_id, cx)
            .finished()
            .iter()
            .map(|w| w.chat_id.clone())
            .filter(|id| *id != self.active_chat)
            .collect();
        for id in ids {
            self.set_worker_archived(&id, true, cx);
        }
        cx.notify();
    }

    /// One `setChatArchived`; failures show in the sidebar notice.
    fn set_worker_archived(&mut self, worker_id: &str, archived: bool, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.sidebar_notice = Some("Engine not connected".into());
            cx.notify();
            return;
        };
        let id = worker_id.to_owned();
        cx.spawn(async move |this, cx| {
            let failed = engine
                .client()
                .call(
                    mast_rpc::methods::MUTATE,
                    serde_json::json!({ "op": "setChatArchived", "chatId": id, "archived": archived }),
                )
                .await
                .is_err();
            if failed {
                let _ = this.update(cx, |this, cx| {
                    this.sidebar_notice = Some("Could not change that chat's archive state. Try again.".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
}
