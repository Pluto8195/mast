//! Comments on HTML pages and the frame bridge (headless gpui, fake frame
//! messages): the markers pushed to the page, selection and element
//! drafts, saving and resolving, the list, and answering question cards in
//! place.
use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use mast_proto::{
    ArtifactComment, ArtifactContentEncoding, ArtifactKind, ArtifactView, CommentAnchor,
    CommentStatus,
};
use mast_rpc::methods;
use serde_json::json;

use super::tab::{ArtifactTab, FakeWebHost, WebProbe};
use super::tests::{Calls, Reply, artifact, content_reply, id, init_ui, scripted, snapshot};

const PAGE: &str = "<h2 id=\"intake\">Receiving intake</h2>\
<p>Parts arrive at the dock.</p>\
<div data-mast-question=\"Q-1\" data-status=\"open\">Merge by MPN?</div>";

const SHEET: &str = r#"{"kind":"decision-sheet","title":"Sheet","flows":[{"id":"D-2","title":"Flow","status":{"state":"open"},"recommendation":"Proceed","question":"Proceed?","steps":[]},{"id":"D-4","title":"Other","optional":true,"status":{"state":"open"},"recommendation":"Proceed","question":"Proceed?","steps":[]}]}"#;

fn chat(id: &str) -> mast_proto::Chat {
    serde_json::from_value(json!({
        "id": id, "deviceId": "dev", "archived": false,
        "createdAt": "2026-07-19T12:00:00Z",
    }))
    .expect("chat")
}

fn comment(id: &str, anchor: CommentAnchor, status: CommentStatus) -> ArtifactComment {
    ArtifactComment {
        id: id.into(),
        quote: anchor.exact.clone(),
        anchor,
        text: format!("note {id}"),
        status,
        revision: 2,
        ..Default::default()
    }
}

fn by_id(value: &str) -> CommentAnchor {
    CommentAnchor {
        id: Some(value.into()),
        exact: "Receiving intake".into(),
        ..Default::default()
    }
}

fn by_quote(value: &str) -> CommentAnchor {
    CommentAnchor {
        exact: value.into(),
        ..Default::default()
    }
}

fn replies(kind: ArtifactKind, page: &'static str) -> Reply {
    Arc::new(move |method, params| match method {
        methods::ARTIFACT_CONTENT => {
            Ok(content_reply(kind, 2, page, ArtifactContentEncoding::Utf8))
        }
        methods::ARTIFACT_DRIFT => Ok(json!({ "artifactId": params["artifactId"] })),
        methods::ARTIFACT_ADD_COMMENT => Ok(json!({
            "artifactId": params["artifactId"],
            "comment": {
                "id": "C-9", "anchor": params["anchor"], "quote": params["quote"],
                "text": params["text"], "status": "open", "revision": 2,
            },
        })),
        methods::ARTIFACT_RESOLVE_COMMENT => Ok(json!({
            "artifactId": params["artifactId"], "commentId": params["commentId"],
            "resolved": params["resolved"],
        })),
        methods::ARTIFACT_ANSWER_QUESTION => Ok(json!({
            "artifactId": params["artifactId"], "questionId": params["questionId"],
        })),
        methods::ARTIFACT_SUBMIT_SHEET_DECISIONS => Ok(json!({
            "artifactId": params["artifactId"],
            "decisions": params["decisions"].as_array().unwrap().iter().map(|d| json!({
                "rowId":d["rowId"], "outcome":d["outcome"], "overrideRecommendation":d["overrideRecommendation"], "note":d["note"], "at":1, "revision":params["revision"]
            })).collect::<Vec<_>>()
        })),
        methods::ARTIFACT_ANSWER_QUESTIONS => Ok(json!({
            "artifactId": params["artifactId"], "answers": params["answers"].as_array().unwrap().iter().map(|a| json!({"id":a["questionId"], "answer":a["answer"], "at":1, "revision":params["revision"]})).collect::<Vec<_>>()
        })),
        methods::CONTINUE_CHAT_FOR_WORK_ITEM => Ok(json!({
            "chatId": params["chatId"], "messageId": "m", "commandId": "k", "state": "started",
        })),
        other => panic!("unexpected {other}"),
    })
}

type Shown = std::rc::Rc<std::cell::RefCell<Vec<(String, String)>>>;

fn settle(vcx: &mut VisualTestContext) {
    for _ in 0..3 {
        vcx.run_until_parked();
        vcx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
    }
}

fn open(
    cx: &mut TestAppContext,
    kind: ArtifactKind,
    comments: Vec<ArtifactComment>,
) -> (
    Entity<ArtifactTab>,
    Calls,
    WebProbe,
    Shown,
    &mut VisualTestContext,
) {
    open_page(cx, kind, comments, if kind == ArtifactKind::DecisionSheet { SHEET } else { PAGE })
}

fn open_page<'a>(
    cx: &'a mut TestAppContext, kind: ArtifactKind, comments: Vec<ArtifactComment>, page: &'static str,
) -> (Entity<ArtifactTab>, Calls, WebProbe, Shown, &'a mut VisualTestContext) {
    let (actions, calls) = scripted(replies(kind, page));
    let shown: Shown = Default::default();
    let probe = WebProbe::default();
    let factory = FakeWebHost::probed_factory(shown.clone(), probe.clone());
    let mut view: ArtifactView = artifact(&id(1), "Flow", kind);
    view.comments = comments;
    let state = cx.new(|_| crate::state::AppState::new());
    state.update(cx, |state, _| {
        state.local_device_id = Some("dev".into());
        state.work_outline_actions_override = Some(actions);
        state.chats = vec![chat("chat-api")];
        state.set_test_artifacts(snapshot(vec![view.clone()]));
    });
    let artifact_id = view.id.clone();
    let (tab, vcx) = cx.add_window_view(move |window, cx| {
        ArtifactTab::new(state, None, artifact_id, factory, window, cx)
    });
    settle(vcx);
    (tab, calls, probe, shown, vcx)
}

fn frame(tab: &Entity<ArtifactTab>, vcx: &mut VisualTestContext, message: serde_json::Value) {
    let json = message.to_string();
    tab.update_in(vcx, |tab, window, cx| {
        tab.on_frame_message(&json, window, cx)
    });
    settle(vcx);
}

fn has(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.debug_bounds(selector).is_some()
}

fn method_calls(calls: &Calls, method: &str) -> Vec<serde_json::Value> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, _)| m == method)
        .map(|(_, params)| params.clone())
        .collect()
}

fn marker_scripts(probe: &WebProbe) -> Vec<String> {
    probe
        .scripts
        .borrow()
        .iter()
        .filter(|script| script.contains("mastComments"))
        .cloned()
        .collect()
}

#[gpui::test]
fn page_gets_markers_only_for_open_found_comments_once_ready(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let comments = vec![
        comment("C-1", by_id("intake"), CommentStatus::Open),
        comment(
            "C-2",
            by_quote("Words that were deleted"),
            CommentStatus::Open,
        ),
        comment("C-3", by_id("intake"), CommentStatus::Resolved),
    ];
    let (tab, _calls, probe, _shown, vcx) = open(cx, ArtifactKind::HtmlFragment, comments);
    // Nothing is pushed before the page says its scripts are listening.
    assert!(marker_scripts(&probe).is_empty());
    frame(&tab, vcx, json!({ "t": "ready" }));
    let scripts = marker_scripts(&probe);
    assert_eq!(scripts.len(), 1, "{scripts:?}");
    assert!(scripts[0].contains("\"C-1\""));
    assert!(!scripts[0].contains("\"C-2\""), "outdated has no marker");
    assert!(!scripts[0].contains("\"C-3\""), "resolved has no marker");
    // Unchanged comments are not pushed again.
    settle(vcx);
    assert_eq!(marker_scripts(&probe).len(), 1);
    // A reloaded page asks again.
    tab.update_in(vcx, |tab, window, cx| {
        let _ = (window, cx);
        tab.comments_document_loading_for_tests();
    });
    frame(&tab, vcx, json!({ "t": "ready" }));
    assert_eq!(marker_scripts(&probe).len(), 2);
}

#[gpui::test]
fn list_shows_outdated_open_and_resolved_rows(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let comments = vec![
        comment("C-1", by_id("intake"), CommentStatus::Open),
        comment(
            "C-2",
            by_quote("Words that were deleted"),
            CommentStatus::Open,
        ),
        comment("C-3", by_id("intake"), CommentStatus::Resolved),
    ];
    let (tab, _calls, _probe, _shown, vcx) = open(cx, ArtifactKind::HtmlFragment, comments);
    assert!(has(vcx, "artifact-comments-toggle"));
    assert!(!has(vcx, "artifact-comments-panel"));
    // A click on a marker in the page opens the list and selects it.
    frame(&tab, vcx, json!({ "t": "focus", "id": "C-1" }));
    assert!(has(vcx, "artifact-comments-panel"));
    for row in [
        "artifact-comment-C-1",
        "artifact-comment-C-2",
        "artifact-comment-C-3",
    ] {
        assert!(has(vcx, row), "{row}");
    }
    tab.update(vcx, |tab, cx| {
        let artifact = tab.artifact(cx).unwrap();
        let states: Vec<_> = tab
            .comment_rows_for_tests(&artifact)
            .into_iter()
            .map(|row| row.state.label())
            .collect();
        assert_eq!(states, ["Open", "Outdated", "Resolved"]);
    });
}

#[gpui::test]
fn selection_becomes_a_saved_comment_and_the_chat_is_told(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (tab, calls, _probe, _shown, vcx) = open(cx, ArtifactKind::HtmlFragment, Vec::new());
    assert!(!has(vcx, "artifact-comment-selection"));
    frame(
        &tab,
        vcx,
        json!({ "t": "selection", "quote": "arrive at the dock", "anchor": {
            "id": "intake", "prefix": "Parts ", "exact": "arrive at the dock", "suffix": "." } }),
    );
    assert!(has(vcx, "artifact-comment-selection"));
    tab.update_in(vcx, |tab, window, cx| tab.comment_on_selection(window, cx));
    settle(vcx);
    assert!(has(vcx, "artifact-comment-draft"));
    tab.update_in(vcx, |tab, _, cx| {
        tab.set_comment_text_for_tests("Which dock?", cx);
        tab.submit_comment(cx);
    });
    settle(vcx);
    let added = method_calls(&calls, methods::ARTIFACT_ADD_COMMENT);
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["text"], "Which dock?");
    assert_eq!(added[0]["anchor"]["id"], "intake");
    assert_eq!(added[0]["anchor"]["exact"], "arrive at the dock");
    assert_eq!(added[0]["quote"], "arrive at the dock");
    // The publishing chat hears about it, in the structured form.
    let sent = method_calls(&calls, methods::CONTINUE_CHAT_FOR_WORK_ITEM);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["chatId"], "chat-api");
    let text = sent[0].to_string();
    assert!(text.contains("Comment C-9 on *Flow* r2"), "{text}");
    assert!(text.contains("Which dock?"), "{text}");
    // The draft closed.
    assert!(!has(vcx, "artifact-comment-draft"));
}

#[gpui::test]
fn picking_an_element_starts_a_draft_and_leaves_pick_mode(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (tab, _calls, probe, _shown, vcx) = open(cx, ArtifactKind::HtmlFragment, Vec::new());
    tab.update_in(vcx, |tab, _, cx| tab.set_pick_mode(true, cx));
    assert!(
        probe
            .scripts
            .borrow()
            .iter()
            .any(|s| s.contains("\"mastCommentMode\":true"))
    );
    frame(
        &tab,
        vcx,
        json!({ "t": "draft", "quote": "Receiving intake",
                "anchor": { "id": "intake", "exact": "Receiving intake" } }),
    );
    assert!(has(vcx, "artifact-comment-draft"));
    assert!(
        probe
            .scripts
            .borrow()
            .iter()
            .any(|s| s.contains("\"mastCommentMode\":false"))
    );
}

#[gpui::test]
fn resolving_and_reopening_call_the_engine(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let comments = vec![comment("C-1", by_id("intake"), CommentStatus::Open)];
    let (tab, calls, _probe, _shown, vcx) = open(cx, ArtifactKind::HtmlFragment, comments);
    tab.update_in(vcx, |tab, _, cx| {
        tab.resolve_comment("C-1".into(), true, cx)
    });
    settle(vcx);
    let resolved = method_calls(&calls, methods::ARTIFACT_RESOLVE_COMMENT);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0]["commentId"], "C-1");
    assert_eq!(resolved[0]["resolved"], true);
}

#[gpui::test]
fn html_question_cards_are_answered_through_the_bridge(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (tab, calls, probe, shown, vcx) = open(cx, ArtifactKind::HtmlFragment, Vec::new());
    frame(&tab, vcx, json!({ "t": "ready" }));
    let loads = shown.borrow().len();
    frame(
        &tab,
        vcx,
        json!({ "t": "answer", "id": "q-1", "text": "  By MPN " }),
    );
    assert!(method_calls(&calls, methods::ARTIFACT_ANSWER_QUESTIONS).is_empty());
    assert!(method_calls(&calls, methods::CONTINUE_CHAT_FOR_WORK_ITEM).is_empty());
    tab.update(vcx, |tab, cx| tab.send_question_batch(cx));
    settle(vcx);
    let answered = method_calls(&calls, methods::ARTIFACT_ANSWER_QUESTIONS);
    assert_eq!(answered.len(), 1, "{answered:?}");
    assert_eq!(answered[0]["answers"][0]["questionId"], "Q-1");
    assert_eq!(answered[0]["answers"][0]["answer"], "By MPN");
    let sent = method_calls(&calls, methods::CONTINUE_CHAT_FOR_WORK_ITEM);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["prompt"]["text"], "Answers for Flow: Q-1: By MPN");
    // The card updates in place: a command to the page, no reload.
    settle(vcx);
    let updates: Vec<_> = probe
        .scripts
        .borrow()
        .iter()
        .filter(|s| s.contains("mastAnswers"))
        .cloned()
        .collect();
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(updates[0].contains("\"Q-1\":\"By MPN\""));
    assert_eq!(shown.borrow().len(), loads, "no reload");
}

#[gpui::test]
fn bad_frame_messages_do_nothing(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (tab, calls, _probe, _shown, vcx) = open(cx, ArtifactKind::HtmlFragment, Vec::new());
    for bad in [
        json!({ "t": "answer", "id": "Q-1", "text": "" }),
        json!({ "t": "answer", "id": "nope", "text": "x" }),
        json!({ "t": "selection", "anchor": {} }),
        json!({ "t": "who" }),
    ] {
        frame(&tab, vcx, bad);
    }
    tab.update_in(vcx, |tab, window, cx| {
        tab.on_frame_message("{not json", window, cx)
    });
    settle(vcx);
    assert!(method_calls(&calls, methods::ARTIFACT_ANSWER_QUESTION).is_empty());
    assert!(!has(vcx, "artifact-comment-selection"));
}

#[gpui::test]
fn markdown_artifacts_have_no_comment_controls(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (_tab, _calls, _probe, _shown, vcx) = open(cx, ArtifactKind::Markdown, Vec::new());
    assert!(!has(vcx, "artifact-comments-toggle"));
    assert!(!has(vcx, "artifact-comment-pick"));
}

#[gpui::test]
fn mermaid_artifacts_take_comments(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (_tab, _calls, _probe, _shown, vcx) = open(cx, ArtifactKind::Mermaid, Vec::new());
    assert!(has(vcx, "artifact-comments-toggle"));
}

#[test]
fn app_documents_carry_the_bridge_and_render_checks_do_not() {
    use super::document::web_document;
    use super::model::WebKind;
    let theme = crate::browser::fragment::FragmentTheme {
        dark: false,
        vars: Vec::new(),
        font_sans: "sans-serif".into(),
        font_mono: "monospace".into(),
    };
    for kind in [WebKind::Fragment, WebKind::Page, WebKind::Mermaid] {
        let live = web_document(kind, PAGE, "Flow", "text/html", &theme, false, false);
        assert!(live.contains("mast-msg:"), "{kind:?} host bridge");
        assert!(
            live.contains("__mastCommentsInstalled"),
            "{kind:?} frame script"
        );
        let probed = web_document(kind, PAGE, "Flow", "text/html", &theme, false, true);
        assert!(
            !probed.contains("__mastCommentsInstalled"),
            "{kind:?} probe stays passive"
        );
    }
}

#[gpui::test]
fn sheet_drafts_stage_undo_and_send_once(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (tab, calls, _probe, shown, vcx) = open(cx, ArtifactKind::DecisionSheet, Vec::new());
    assert!(shown.borrow().last().unwrap().1.contains("sheet-stage"));
    frame(&tab, vcx, json!({"t":"sheet-stage","id":"D-2","outcome":"decided","note":""}));
    frame(&tab, vcx, json!({"t":"sheet-stage","id":"D-4","outcome":"rejected","note":"too slow"}));
    assert!(method_calls(&calls, methods::ARTIFACT_SUBMIT_SHEET_DECISIONS).is_empty());
    assert!(method_calls(&calls, methods::CONTINUE_CHAT_FOR_WORK_ITEM).is_empty());
    vcx.update(|_, cx| assert_eq!(crate::settings::current(cx).artifact_decision_drafts[&id(1)].decisions.len(), 2));
    frame(&tab, vcx, json!({"t":"sheet-undo","id":"D-2"}));
    vcx.update(|_, cx| assert_eq!(crate::settings::current(cx).artifact_decision_drafts[&id(1)].decisions.len(), 1));
    frame(&tab, vcx, json!({"t":"sheet-stage","id":"D-2","outcome":"decided","note":"Use UTC","overrideRecommendation":true}));
    frame(&tab, vcx, json!({"t":"sheet-send"}));
    assert_eq!(method_calls(&calls, methods::ARTIFACT_SUBMIT_SHEET_DECISIONS).len(), 1);
    let sent = method_calls(&calls, methods::CONTINUE_CHAT_FOR_WORK_ITEM);
    assert_eq!(sent.len(), 1);
    let message = sent[0].to_string();
    assert!(message.contains("D-2 answered: Use UTC"));
    assert!(message.contains("D-4 rejected (too slow)"));
    vcx.update(|_, cx| assert!(!crate::settings::current(cx).artifact_decision_drafts.contains_key(&id(1))));
}

#[gpui::test]
fn sheet_drafts_survive_reopening(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    {
        let (tab, _calls, _probe, _shown, vcx) = open(cx, ArtifactKind::DecisionSheet, Vec::new());
        frame(&tab, vcx, json!({"t":"sheet-stage","id":"D-2","outcome":"decided","note":"keep this"}));
    }
    let (_tab, _calls, _probe, shown, vcx) = open(cx, ArtifactKind::DecisionSheet, Vec::new());
    let document = shown.borrow().last().unwrap().1.clone();
    assert!(document.contains("keep this"));
    vcx.update(|_, cx| assert_eq!(crate::settings::current(cx).artifact_decision_drafts[&id(1)].decisions[0].note.as_deref(), Some("keep this")));
}

#[gpui::test]
fn real_sheet_stages_and_sends(cx: &mut TestAppContext) {
    let dir = init_ui(cx);
    let Ok(path) = std::env::var("MAST_REAL_SHEET_TEST") else { return; };
    let page: &'static str = Box::leak(std::fs::read_to_string(path).unwrap().into_boxed_str());
    let (tab, calls, _probe, shown, vcx) = open_page(cx, ArtifactKind::DecisionSheet, Vec::new(), page);
    assert!(shown.borrow().last().unwrap().1.contains("sheet-stage"));
    for id in ["flow-release-hold", "ui-draft", "ui-partial", "rule-move-first"] {
        frame(&tab, vcx, json!({"t":"sheet-stage","id":id,"outcome":"decided","note":""}));
        vcx.update(|_, cx| assert!(crate::settings::current(cx).artifact_decision_drafts.get(&super::tests::id(1)).is_some_and(|d| d.decisions.iter().any(|d| d.row_id == id)), "{id}"));
    }
    let settings: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings/ui-settings.json")).unwrap()).unwrap();
    assert_eq!(settings["artifactDecisionDrafts"][id(1)]["decisions"].as_array().unwrap().len(), 4);
    frame(&tab, vcx, json!({"t":"sheet-send"}));
    assert_eq!(method_calls(&calls, methods::CONTINUE_CHAT_FOR_WORK_ITEM).len(), 1);
}

#[gpui::test]
fn sheet_refusals_notify_the_tab_and_frame(cx: &mut TestAppContext) {
    let _dir = init_ui(cx);
    let (tab, _calls, probe, _shown, vcx) = open(cx, ArtifactKind::DecisionSheet, Vec::new());
    frame(&tab, vcx, json!({"t":"sheet-stage","id":"missing","outcome":"decided","note":""}));
    assert!(tab.read_with(vcx, |tab, _| tab.notice().unwrap().contains("no longer open")));
    assert!(probe.scripts.borrow().iter().any(|s| s.contains("mastSheetStageRejected") && s.contains("missing")));
    frame(&tab, vcx, json!({"t":"sheet-send"}));
    assert!(tab.read_with(vcx, |tab, _| tab.notice().unwrap().contains("no staged decisions")));
    assert!(probe.scripts.borrow().iter().any(|s| s.contains("mastSheetEnableSend")));
}
