//! Coordinator hand-off dialog.
use super::*;
use super::work_outline_ui::WorkOutlineModal;
use crate::work_outline::actions::OperationSlot;
use crate::pickers::Pickers;

pub(super) struct HandOffDialog {
    chat_id: String,
    input: Entity<ComposerInput>,
    pickers: Entity<Pickers>,
    _picker_observe: Subscription,
    workers: Vec<String>,
    details_available: bool,
    archive: bool,
    submitting: bool,
    error: Option<String>,
    operation: OperationSlot,
}

impl HandOffDialog {
    pub(super) fn is_submitting(&self) -> bool { self.submitting }
    pub(super) fn input_focus(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Shell {
    #[cfg(feature = "snapshot")]
    pub(crate) fn fixture_snapshot_hand_off(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, _| {
            if let Some(worker) = state.chats.iter_mut().find(|chat| chat.id == "fixture-design") {
                worker.parent_chat_id = Some("fixture-chat".into());
            }
        });
        self.open_hand_off("fixture-chat".into(), cx);
    }

    pub(super) fn hand_off_pickers(&self) -> Option<Entity<Pickers>> {
        match self.work_outline_modal.as_ref() {
            Some(WorkOutlineModal::HandOff(dialog)) => Some(dialog.pickers.clone()),
            _ => None,
        }
    }

    pub(super) fn has_hand_off_workers(&self, chat_id: &str, cx: &App) -> bool {
        self.state.read(cx).chats.iter().any(|chat| !chat.archived && chat.parent_chat_id.as_deref() == Some(chat_id))
    }

    pub(super) fn open_hand_off(&mut self, chat_id: String, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(chat) = state.chats.iter().find(|chat| chat.id == chat_id) else { return };
        let load = chat.space_id.as_deref().map(|space_id| state.work_outline(space_id));
        let snapshot = load.as_ref().and_then(|load| load.snapshot());
        let item = snapshot.and_then(|snapshot| crate::work_outline::instructions::assigned_chats(snapshot).remove(&chat_id)
            .and_then(|id| snapshot.item(&id)));
        let summary = format!("Continue as coordinator for {}. Read the work item, its instructions, and the worker list below.", item.map(|item| item.title.as_str()).unwrap_or("this work item"));
        let details_available = item.is_some();
        let config = chat.config.clone();
        let device = chat.space_id.as_deref().and_then(|id| state.space_row(id)).map(|space| space.device_id.clone());
        let workers: Vec<String> = state.chats.iter().filter(|worker| !worker.archived && worker.parent_chat_id.as_deref() == Some(chat_id.as_str())).map(|worker| {
            let status = state.sessions.iter().find(|session| session.chat_id == worker.id)
                .map(|session| match session.status {
                    mast_proto::SessionStatus::Idle => "Idle",
                    mast_proto::SessionStatus::Working => "Working",
                    mast_proto::SessionStatus::AwaitingInput => "Awaiting input",
                    mast_proto::SessionStatus::Errored => "Errored",
                }).unwrap_or("Unknown");
            let requests = snapshot.map(|snapshot| snapshot.user_action_requests.iter().filter(|request| request.chat_id == worker.id && request.is_unresolved() && !request.deduped).count());
            let requests = requests.map(|count| format!("{count} open requests")).unwrap_or_else(|| "Request count unavailable".into());
            format!("{} · {} · {}", worker.title.as_deref().unwrap_or("Untitled"), status, requests)
        }).collect();
        if workers.is_empty() { return; }
        let composer_pickers = self.composer.read(cx).pickers().clone();
        let pickers = cx.new(|cx| {
            let mut pickers = Pickers::new_chat_launcher(&composer_pickers, device, cx);
            pickers.seed_hand_off(config.as_ref(), cx);
            pickers
        });
        let picker_observe = cx.observe(&pickers, |_, _, cx| cx.notify());
        let input = cx.new(|cx| {
            let mut input = ComposerInput::new("Summary", cx).with_text_metrics(12.5, 18.0).with_auto_grow();
            input.set_text(summary, cx);
            input
        });
        if !details_available {
            // Selecting the source makes its Space's outline watch active.
            self.open_chat(chat_id.clone(), cx);
        }
        self.begin_work_outline_modal(WorkOutlineModal::HandOff(HandOffDialog {
            chat_id, input, pickers, _picker_observe: picker_observe, workers, details_available, archive: false, submitting: false,
            error: (!details_available).then(|| "Work item details are unavailable. Open the chat and try again.".into()),
            operation: OperationSlot::default(),
        }), cx);
    }

    pub(super) fn refresh_hand_off_details(&mut self, cx: &mut Context<Self>) {
        let Some(WorkOutlineModal::HandOff(dialog)) = self.work_outline_modal.as_ref() else { return };
        if dialog.details_available { return; }
        let chat_id = dialog.chat_id.clone();
        let state = self.state.read(cx);
        let Some(space_id) = state.chats.iter().find(|chat| chat.id == chat_id).and_then(|chat| chat.space_id.as_deref()) else { return };
        let load = state.work_outline(space_id);
        let Some(snapshot) = load.snapshot() else { return };
        let Some(item) = crate::work_outline::instructions::assigned_chats(snapshot).remove(&chat_id).and_then(|id| snapshot.item(&id)) else { return };
        let summary = format!("Continue as coordinator for {}. Read the work item, its instructions, and the worker list below.", item.title);
        let workers = state.chats.iter().filter(|worker| !worker.archived && worker.parent_chat_id.as_deref() == Some(chat_id.as_str())).map(|worker| {
            let status = state.sessions.iter().find(|session| session.chat_id == worker.id).map(|session| match session.status {
                mast_proto::SessionStatus::Idle => "Idle", mast_proto::SessionStatus::Working => "Working",
                mast_proto::SessionStatus::AwaitingInput => "Awaiting input", mast_proto::SessionStatus::Errored => "Errored",
            }).unwrap_or("Unknown");
            let requests = snapshot.user_action_requests.iter().filter(|request| request.chat_id == worker.id && request.is_unresolved() && !request.deduped).count();
            format!("{} · {} · {} open requests", worker.title.as_deref().unwrap_or("Untitled"), status, requests)
        }).collect();
        let Some(WorkOutlineModal::HandOff(dialog)) = self.work_outline_modal.as_mut() else { return };
        dialog.workers = workers;
        dialog.details_available = true;
        dialog.error = None;
        dialog.input.update(cx, |input, cx| {
            if input.text() == "Continue as coordinator for this work item. Read the work item, its instructions, and the worker list below." {
                input.set_text(summary, cx);
            }
        });
    }

    fn submit_hand_off(&mut self, cx: &mut Context<Self>) {
        let actions = self.state.read(cx).work_outline_actions();
        let Some(WorkOutlineModal::HandOff(dialog)) = self.work_outline_modal.as_mut() else { return };
        if dialog.submitting { return; }
        if !dialog.details_available {
            dialog.error = Some("Work item details are unavailable. Open the chat and try again.".into());
            cx.notify();
            return;
        }
        let summary = dialog.input.read(cx).text().trim().to_owned();
        if summary.is_empty() { dialog.error = Some("Enter a summary".into()); cx.notify(); return; }
        let source = dialog.chat_id.clone();
        let choice = dialog.pickers.read(cx).resolved(cx);
        let harness = choice.harness;
        let model = choice.model;
        let reasoning = choice.reasoning;
        let operation_id = dialog.operation.id_for(&format!("{summary}|{harness:?}|{model:?}|{reasoning:?}"));
        let task = actions.hand_off_orchestrator(mast_proto::HandOffOrchestratorParams { chat_id: source.clone(), summary, operation_id, harness, model, reasoning });
        dialog.submitting = true;
        dialog.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let Some(WorkOutlineModal::HandOff(dialog)) = this.work_outline_modal.as_mut() else { return };
                if dialog.chat_id != source { return; }
                dialog.submitting = false;
                match result {
                    Ok(reply) if reply.state != mast_proto::WorkItemLaunchState::Rejected => {
                        let archive = dialog.archive;
                        this.work_outline_modal = None;
                        this.work_outline_modal_focus_pending = false;
                        this.work_outline_prior_focus = None;
                        if archive { this.archive_chat(source, cx); }
                        this.open_chat(reply.chat_id, cx);
                    }
                    Ok(reply) => { dialog.error = Some(reply.detail.unwrap_or_else(|| "The host rejected the first turn".into())); cx.notify(); }
                    Err(error) => { dialog.error = Some(error.message); cx.notify(); }
                }
            }).ok();
        }).detach();
    }
}

pub(super) fn hand_off_card(dialog: &HandOffDialog, theme: &Theme, cx: &mut Context<Shell>) -> AnyElement {
    let mut workers = div().flex().flex_col().gap(px(4.0));
    for worker in &dialog.workers { workers = workers.child(div().child(SharedString::from(worker.clone()))); }
    popover::dialog_card(theme)
        .debug_selector(|| "hand-off-dialog".into())
        .child(popover::dialog_title(theme, "Hand off to new chat"))
        .child(div().mt(px(12.0)).child(popover::dialog_field(&theme, dialog.input.clone().into_any_element())))
        .child(div().mt(px(12.0)).child(dialog.pickers.clone()))
        .child(div().mt(px(12.0)).child(SharedString::from("Workers")))
        .child(div().id("hand-off-workers").max_h(px(180.0)).overflow_y_scroll().text_color(theme.text_muted).child(workers))
        .child(div().id("hand-off-archive").mt(px(12.0)).cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
            if let Some(WorkOutlineModal::HandOff(dialog)) = this.work_outline_modal.as_mut() { dialog.archive = !dialog.archive; cx.notify(); }
        })).child(SharedString::from(if dialog.archive { "☑ Archive this chat after hand-off" } else { "☐ Archive this chat after hand-off" })))
        .when_some(dialog.error.clone(), |card, error| card.child(div().mt(px(8.0)).text_color(theme.warning).child(SharedString::from(error))))
        .child(div().mt(px(16.0)).flex().justify_end().gap(px(8.0))
            .child(popover::btn_ghost(theme, "Cancel", "hand-off-cancel").id("hand-off-cancel").on_click(cx.listener(|this, _, window, cx| { this.cancel_work_outline_modal(window, cx); })))
            .child(popover::btn_primary(theme, if dialog.submitting { "Handing off…" } else { "Hand off" }).id("hand-off-submit").on_click(cx.listener(|this, _, _, cx| this.submit_hand_off(cx)))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::work_outline::store::fixture_ids::{CHAT_API, CHAT_UI};
    use std::sync::{Arc, Mutex};

    #[gpui::test]
    fn dialog_lists_live_workers_and_prefills_summary(cx: &mut gpui::TestAppContext) {
        let (shell, cx) = super::super::chat_item_tests::mount(cx, CHAT_API, false, |_| {});
        shell.update(cx, |shell, cx| {
            assert!(!shell.has_hand_off_workers(CHAT_API, cx));
            shell.state.update(cx, |state, _| {
                let worker = state.chats.iter_mut().find(|chat| chat.id == CHAT_UI).unwrap();
                worker.parent_chat_id = Some(CHAT_API.into());
            });
            assert!(shell.has_hand_off_workers(CHAT_API, cx));
            shell.open_hand_off(CHAT_API.into(), cx);
            let Some(WorkOutlineModal::HandOff(dialog)) = &shell.work_outline_modal else { panic!("hand-off dialog not opened") };
            assert!(dialog.input.read(cx).text().starts_with("Continue as coordinator for "));
            assert!(dialog.input.read(cx).text().ends_with("Read the work item, its instructions, and the worker list below."));
            assert_eq!(dialog.workers.len(), 1);
            assert!(dialog.workers[0].contains("Title of"));
            assert!(dialog.workers[0].contains("0 open requests"));
            assert!(!dialog.archive);
        });
    }

    #[gpui::test]
    fn dialog_refreshes_after_outline_arrives(cx: &mut gpui::TestAppContext) {
        let (shell, cx) = super::super::chat_item_tests::mount(cx, CHAT_API, false, |_| {});
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.iter_mut().find(|chat| chat.id == CHAT_UI).unwrap().parent_chat_id = Some(CHAT_API.into());
                state.set_work_outline_for_test(mast_proto::WorkOutlineSnapshot::unavailable("s1", false, 0));
            });
            shell.open_hand_off(CHAT_API.into(), cx);
            let Some(WorkOutlineModal::HandOff(dialog)) = &shell.work_outline_modal else { panic!() };
            assert!(!dialog.details_available);
            shell.state.update(cx, |state, _| state.set_work_outline_for_test(crate::work_outline::store::fixture_snapshot("s1")));
            shell.refresh_hand_off_details(cx);
            let Some(WorkOutlineModal::HandOff(dialog)) = &shell.work_outline_modal else { panic!() };
            assert!(dialog.details_available);
            assert!(dialog.error.is_none());
            assert!(dialog.workers[0].contains("0 open requests"));
        });
    }

    #[gpui::test]
    fn submit_sends_summary_and_archives_after_success(cx: &mut gpui::TestAppContext) {
        let (shell, cx) = super::super::chat_item_tests::mount(cx, CHAT_API, false, |_| {});
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let actions = crate::work_outline::actions::WorkOutlineActions::scripted(Arc::new(move |method, params| {
            recorded.lock().unwrap().push((method.to_owned(), params));
            Ok(serde_json::json!({"chatId":"successor", "messageId":"m", "commandId":"c", "executionRoot":"/work", "state":"queued"}))
        }));
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.iter_mut().find(|chat| chat.id == CHAT_UI).unwrap().parent_chat_id = Some(CHAT_API.into());
                state.chats.iter_mut().find(|chat| chat.id == CHAT_API).unwrap().config = Some(mast_proto::ChatConfig {
                    harness: mast_proto::HarnessId::Codex, model: Some("source-model".into()), reasoning: None,
                    model_options: Default::default(), sandbox: mast_proto::SandboxLevel::WorkspaceWrite, auto_approve: false,
                });
                state.work_outline_actions_override = Some(actions);
            });
            shell.open_hand_off(CHAT_API.into(), cx);
            let Some(WorkOutlineModal::HandOff(dialog)) = shell.work_outline_modal.as_mut() else { panic!() };
            dialog.pickers.update(cx, |pickers, cx| {
                let selected = pickers.resolved(cx);
                assert_eq!(selected.harness, Some(mast_proto::HarnessId::Codex));
                assert_eq!(selected.model.as_deref(), Some("source-model"));
                pickers.pick_harness(mast_proto::HarnessId::ClaudeCode, cx);
                pickers.pick_model_for_tests("claude-opus-5-5", cx);
            });
            dialog.input.update(cx, |input, cx| input.set_text("Continue coordinating", cx));
            dialog.archive = true;
            shell.submit_hand_off(cx);
        });
        cx.run_until_parked();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, mast_rpc::methods::HAND_OFF_ORCHESTRATOR);
        assert_eq!(calls[0].1["chatId"], CHAT_API);
        assert_eq!(calls[0].1["summary"], "Continue coordinating");
        assert_eq!(calls[0].1["harness"], "claude-code");
        assert_eq!(calls[0].1["model"], "claude-opus-5-5");
        assert!(calls[0].1["operationId"].as_str().is_some());
        shell.read_with(cx, |shell, _| {
            assert!(shell.work_outline_modal.is_none());
            assert!(shell.mutations.iter().any(|mutation| mutation["op"] == "setChatArchived" && mutation["chatId"] == CHAT_API && mutation["archived"] == true));
        });
    }

    #[gpui::test]
    fn submit_error_keeps_dialog_open(cx: &mut gpui::TestAppContext) {
        let (shell, cx) = super::super::chat_item_tests::mount(cx, CHAT_API, false, |_| {});
        let actions = crate::work_outline::actions::WorkOutlineActions::scripted(Arc::new(|_, _| Err(mast_rpc::RpcError::Failed("transfer failed".into()))));
        shell.update(cx, |shell, cx| {
            shell.state.update(cx, |state, _| {
                state.chats.iter_mut().find(|chat| chat.id == CHAT_UI).unwrap().parent_chat_id = Some(CHAT_API.into());
                state.work_outline_actions_override = Some(actions);
            });
            shell.open_hand_off(CHAT_API.into(), cx);
            shell.submit_hand_off(cx);
        });
        cx.run_until_parked();
        shell.read_with(cx, |shell, _| {
            let Some(WorkOutlineModal::HandOff(dialog)) = &shell.work_outline_modal else { panic!() };
            assert!(dialog.error.as_deref().unwrap().contains("transfer failed"));
            assert!(!dialog.submitting);
        });
    }
}
