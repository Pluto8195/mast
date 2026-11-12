# Orchestrator wake research

Research-only snapshot; no build, API calls, or live profile writes. Results below are from the supplied kit snapshot and decoded coordinator and worker transcripts. The Mast repository is public, so project content and identifiers are omitted.

## What was measured

The coordinator journal maps 15 session-start events to one Claude Code session. The decoded coordinator transcript has 52 human messages and 61 system wake messages. The 61 wakes contain 67 assistant text entries, including intermediate messages. Thus the earlier report's 67 is a count of replies, not independent wake triggers. The journal records 115 finished or interrupted runs and 113 usage events.

Matching completed journal `done.result` text to decoded replies attributes 61 completed wake-triggered runs and 59,991 output tokens, and 49 completed human-triggered runs and 113,407 output tokens. Five interrupted runs remain separately unattributed: three have usage records (713, 1,581 and 0 output tokens); two have none. A human message can interrupt a prior run; the 52 human messages therefore do not imply 52 completed runs. These are journal-reported outputs, not a complete bill. A hand-off category is not established from the decoded coordinator transcript and must not be assumed to be zero. The journal's usage records expose input and output tokens, but no cache-read token field. The allowed snapshot does not contain the Claude Code session transcript. The standing instruction forbids accessing live Claude sessions. A read-only copy outside that directory is needed to measure cache reads, classify hand-off costs, and plot context growth. Output counts alone cannot distinguish wake frequency from growing-context cost.

## A and B replay

Treat each of the 61 historical wake messages as a batch; a batch is held only when all its events are informational. Existing untagged reports have no status, so classifications below are counterfactual, not recorded worker intent. A conservative content review identifies five report-only batches that could have been explicitly labeled progress (wake ordinals 5, 14, 51, 52, 55). A would hold these five, whose completed runs used 1,174 output tokens. Acknowledgements and progress may still contain useful coordination information; holding them delays that information. Two additional report-only batches (10 and 60) describe incomplete work but also a failed capture or an outstanding decision; labeling them progress would raise A to seven and 3,891 output tokens, but is not assumed safe.

B independently holds three resolved-action-only batches (25, 42, 59), associated with 341 output tokens. A plus B therefore holds eight of the 61 historical batches under the conservative labels, accounting for eight of the 67 assistant text entries. A less conservative interpretation holds ten batches and twelve assistant entries. This is a static batch counterfactual, not a simulation of a changed schedule: later arrivals and user messages could combine held events, and the unrecorded new worker behavior may differ. Report-only batches that contain finished results, decisions, or an explicit request remain actionable. Tool-call failures and awaiting input remain actionable; they must not be accidentally held with resolved actions.

## Auto-continue and safety

The available project metrics report zero auto-continue messages and zero exhausted-continuation wakes, a lower bound because decoded chat snapshots may contain transcript tails. Neither the historical setting nor the persisted per-worker counters are in the provided snapshot. The source defaults auto-continue off; this is not proof of the historical configuration. Therefore the evidence cannot distinguish off from eligibility not met for all nine reported nudges, and cannot establish a cap. Do not describe the nine as caught or missed by a particular setting without its historical state and turn-by-turn signals.

The current eligibility check excludes open actions, waiting items, incomplete turns, awaiting input, tool failures, a user message during the turn and a nonempty queue, and stops after three consecutive continuations. Five inspected stopped-turn examples illustrate why text-only question detection is insufficient:

| Sample | Last message | Visible question? | Relevant condition |
| --- | --- | --- | --- |
| Planning partial draft | Incomplete research; render failed; a brief-confirmation request was open. | No | Open request and failed tool call exclude automatic continuation under current code. |
| Design fixture audit | Incomplete research and no review request. | No | More work remained; no obvious risky step in the final text. |
| Design fixture changes | Incomplete work and browser-profile capture failure. | No | A retry needs environmental repair; a generic nudge may repeat a failure. |
| Decision-sheet audit | Incomplete, not published; earlier draft provenance and code claims unverified. | No | A blind instruction to publish could promote unverified decisions. |
| UI implementation | Incomplete scope; customer-facing fixture values explicitly await a decision. | No | A generic continue could be safe only if it preserves the decision boundary; a previous coordinator instruction to use example values was caught as unsafe. |

These examples include untagged reports; rule C's no-report turn trigger and its untagged-report trigger are distinct. The snapshot does not prove the last message alone captures all requests or queued state. A rule that nudges whenever the last message lacks a question would have failed to detect the decision and failure conditions above. Retain explicit state guards and require safe fallback to the coordinator.

## API and integration

[TypeSafe System One](https://docs.typesafe.ai/api/system-one.md) uses bearer authentication, `POST /v1/systemone`, and a body containing `model`, `state` and named `questions`; discover the `jev-latest` alias with authenticated `GET /v1/models`. A [Choice](https://docs.typesafe.ai/primitives/choice.md) question uses `type: choice`, `instructions` and a `criteria` map; its answer contains `choice`, `probabilities` and `confidence`. [State](https://docs.typesafe.ai/concepts/state.md) supports a JSON object. [Confidence](https://docs.typesafe.ai/confidence) summarizes probability concentration, not measured correctness.

Sketch: send a bounded object with last worker message, item title/status, open requests, and last coordinator instruction. Ask one Choice: “What best describes why this worker turn ended?” Criteria: `stopped_early` (unfinished and able to proceed), `done_unreported` (finished but no handoff), `blocked_needs_decision`, `waiting_on_approval`, `progress_only`, and `other` (none fits). Define boundary examples for each. No model call was made; a human-labeled sample alone cannot select a calibrated confidence threshold. Any eventual test must compare predictions and confidence against held-out labels, and the integration must use fake HTTP in tests, store the key in the existing secret mechanism, bound total latency and preserve a fail-open coordinator wake.

`WakeCore` already persists held batches and protects user priority; settings currently filter both recording and restoring. Separate event enablement from wake eligibility without bypassing per-kind settings, dedupe, durable receipts, cancellation or user-message ordering. `report_to_parent` must carry status across MCP, RPC, proto and persisted events. The current 45-second batch window and 120-second recent-user hold must not independently flush an informational-only batch. Deliver held updates along with the next user message without creating an extra wake.

Project-scoped read reach exists in `crates/mcp/src/tools.rs`; `send_message` is restricted to self, children and parent. Restrict expanded sends to the same project and Space while preserving parent report semantics. The plan-question reconciler chooses the latest live revision publisher for new requests, but UI sheet and plan answer submission in `comments_view.rs` and `questions_view.rs` sends directly to `artifact.chat_id`, the first publisher. Existing open requests also retain their original recipient. Route these paths consistently to a current owner; do not silently treat a pinned plan flag as a chat owner.

`work_outline_read_subtree` currently includes completed items, excludes archived items by default and caps results at 300. A default omission count needs a defined relationship to depth, archive filtering and truncation; preserve the existing archived option. No token savings for trimming the outline are measured here.

## Recommendation, separate branches

1. **A and B together:** the replay identifies eight conservative historical batches (1,515 output tokens), or ten under a less conservative status assumption. Keep A and B independently reviewable if their changes can be separated cleanly; either way preserve explicit actionable reports and user-triggered delivery. Cache-read savings remain unmeasured.
2. **D and E:** fix project reach and answer routing as separate branches. The snapshot directly shows relay and concurrent-editor friction, but does not quantify token savings.
3. **F:** trim outline reads on its own branch after measuring actual response size and call frequency. Its potential savings are unmeasured and depend on whether the coordinator actually reads the omitted rows.
4. **C:** defer Jev until a labeled and model-scored sample establishes useful precision and latency. The present sample shows a no-question heuristic is unsafe, and there is no measured confidence threshold or savings estimate.

Ranking after A/B is by demonstrated operational friction, not measured token savings; the missing cache-read data prevents a defensible savings ranking across the proposals.
