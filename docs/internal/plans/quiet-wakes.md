# Orchestrator wake research

Research-only snapshot; no build, API calls, or live profile writes. Results below are from the supplied kit snapshot and decoded coordinator and worker transcripts. The Mast repository is public, so project content and identifiers are omitted.

## What was measured

The snapshot journal maps the original coordinator to one Claude Code session. The decoded transcript at the 18:40 UTC snapshot cutoff contains 52 human messages and 61 wake messages. Those wakes contain 67 assistant text entries, including intermediate messages. The earlier 67-reply figure is therefore not 67 independent wake triggers.

The supplied read-only Claude Code transcripts include the original coordinator and a successor hand-off session. SDK user prompts identify turn triggers; tool results and injected skill text are not turns. Assistant usage is summed once per `(message.id, requestId)`, including interrupted turns. Context is `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` for an individual model request, not an accumulated conversation size. The supplied successor transcript begins after the snapshot cutoff, so the two scopes must not be mixed:

| Scope / trigger | Turns | Output tokens | Cache-read tokens |
| --- | ---: | ---: | ---: |
| Snapshot: human | 52 | 115,701 | 67,120,317 |
| Snapshot: wake | 61 | 59,991 | 44,283,975 |
| Snapshot: hand-off | 0 | 0 | 0 |
| Both supplied transcripts: human | 60 | 135,404 | 78,628,694 |
| Both supplied transcripts: wake | 67 | 63,664 | 49,106,958 |
| Both supplied transcripts: hand-off | 1 | 3,610 | 490,584 |

The snapshot wake share is 40% of cache reads and 34% of output tokens. Its original-session first-request context rose from 36,655 tokens in the first turn to 580,264 in the last; maximum observed request context was 583,211. Across the first ten wakes, median first-request context was 110,512 and mean cache reads per wake 312,892; across the last ten, those rose to 566,031 and 1,251,485. Both the number of wakes and the growing context contribute; these observational totals do not isolate causal cost. The successor hand-off starts a fresh session, with 41,100 tokens of first-request context. Cache creation and uncached input are outside the cache-read totals above.

## A and B replay

Treat each of the 61 historical wake messages as a batch; a batch is held only when all its events are informational. Existing untagged reports have no status, so classifications below are counterfactual, not recorded worker intent. A conservative content review identifies five report-only batches that could have been explicitly labeled progress (wake ordinals 5, 14, 51, 52, 55). A would hold these five, whose turns used 1,174 output and 2,327,525 cache-read tokens. Acknowledgements and progress may still contain useful coordination information; holding them delays that information. Two additional report-only batches (10 and 60) describe incomplete work but also a failed capture or an outstanding decision; labeling them progress would raise A to seven and 3,891 output tokens, but is not assumed safe.

B independently holds three resolved-action-only batches (25, 42, 59), associated with 341 output and 1,644,869 cache-read tokens. A plus B therefore holds eight of the 61 historical batches under the conservative labels, accounting for eight of the 67 assistant text entries. A less conservative interpretation holds ten batches and twelve assistant entries. This is a static batch counterfactual, not a simulation of a changed schedule: later arrivals and user messages could combine held events, and the unrecorded new worker behavior may differ. Report-only batches that contain finished results, decisions, or an explicit request remain actionable. Tool-call failures and awaiting input remain actionable; they must not be accidentally held with resolved actions.

## Auto-continue and safety

The available project metrics report zero auto-continue messages and zero exhausted-continuation wakes, a lower bound because decoded chat snapshots may contain transcript tails. Neither the historical setting nor the persisted per-worker counters are in the provided snapshot. The source defaults auto-continue off; this is not proof of the historical configuration. Therefore the evidence cannot distinguish off from eligibility not met for all nine reported nudges, and cannot establish a cap. Do not describe the nine as caught or missed by a particular setting without its historical state and turn-by-turn signals.

Nine candidate coordinator nudges can be traced to the snapshot: planning partial draft (Nov 9 16:19 UTC); design fixture audit, failed capture, and follow-up correction (Nov 10 18:39, 18:42 and 18:50); decision-sheet audit (Nov 10 21:48); and four UI-worker continuations or priority resets (Nov 12 16:39, 17:33, 18:05 and 18:29). These are four distinct chat IDs, while the friction report says five workers; the report does not list all nine, so the match is provisional. For the planning case, a failed Mast tool and open request were present; for the first UI continuation a worker request was open. Either condition blocks the current auto-continue eligibility check even if enabled. The fixture follow-up involved a review action, and the sheet audit mentioned open decisions, but the snapshot does not establish whether those actions belonged to the worker chat at the precise turn boundary. The remaining cases lack sufficient historical setting, queue, failure and item-state evidence to distinguish off, ineligible, or not triggered. None shows evidence of hitting the cap. A prioritization change is also not necessarily a stall.

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

1. **A:** use a separate branch for typed reports; five conservative historical batches used 1,174 output and 2,327,525 cache-read tokens. Preserve explicit actionable reports and user-triggered delivery.
2. **B:** use a separate branch for informational events; three resolved-only batches used 341 output and 1,644,869 cache-read tokens. Preserve per-kind settings.
3. **D and E:** use separate branches for project reach and answer routing. The snapshot directly shows relay and concurrent-editor friction, but does not quantify token savings.
4. **F:** use a separate branch to trim outline reads after measuring response size and call frequency. Potential savings are unmeasured and depend on whether the coordinator reads the omitted rows.
5. **C:** defer Jev until a labeled and model-scored sample establishes useful precision and latency; if pursued, use its own branch. The present sample shows a no-question heuristic is unsafe, and there is no measured confidence threshold or savings estimate.

Ranking after A/B is by demonstrated operational friction, not measured token savings; no comparable savings have been measured for D, E, F or C.
