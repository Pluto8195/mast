# Orchestrator wake research

Status: preliminary. The per-wake replay and stall sample are not complete; do not use these observations as an estimate of the feature's total effect.

## Observations

The supplied decoded coordinator transcript contains 61 `Worker updates:` messages. The friction report's 67 is a count of replies following wakes, a different unit. Three batches contain only resolved-action events. These three would be held by rule B if no previously held actionable event triggered delivery; a full chronological replay is still needed to account for changed batching and subsequent user messages. Historical worker reports have no typed status, so rule A cannot be counted without explicitly labeling their contents and stating that the labels are counterfactual.

## Design direction

- `WakeSettings::wakes_for` currently combines event enablement with wake eligibility; `WakeCore::set_settings` drops events when their kind is disabled. Separate enabled, held, and triggering predicates without changing the existing per-kind opt-outs. Persist held events and deliver them with the next actionable batch or user message, retaining the existing user-first and cancellation/receipt behavior.
- Extend report arguments through MCP, RPC, proto, and the persisted wake event with an optional status. Explicit progress is informational; done, blocked, and question are actionable. Untagged reports retain current behavior when triage is off. Suggest STATUS / RESULT / NEXT in the tool description.
- The current auto-continue decision in `orchestrator_wakes/auto_continue.rs` checks an open, non-waiting item, open actions, completed turn, pending input, failures, and concurrent user activity; it tracks three continuations per worker and item. Preserve these exclusions and recheck state before acting after network I/O. Record decisions durably in the chat. Fail closed to the existing wake on missing key, invalid response, error, or deadline.
- TypeSafe's [System One API](https://docs.typesafe.ai/api/system-one.md) accepts bearer authentication and `model`, `state`, and named `questions`; [Choice](https://docs.typesafe.ai/primitives/choice.md) returns a choice, probabilities, and confidence. Discover `jev-latest` with [models](https://docs.typesafe.ai/api/list-models). The [state](https://docs.typesafe.ai/concepts/state.md) can be a named object. [Confidence](https://docs.typesafe.ai/confidence) is derived from the distribution, not a correctness guarantee. Human labels alone cannot calibrate a threshold: compare model results with a representative labeled sample before choosing one. Keep triage disabled by default and avoid transmitting more context than needed.
- `send_message` currently uses `Reach::SelfChildOrParent`; project-scoped read reach exists separately. Add project-subtree send authorization with the same Space boundary and retain parent-report semantics.
- Trace artifact answer routing and define deterministic current ownership including a pinned owner before changing delivery. Preserve attribution of the user's own answer.
- Add `includeCompleted` to subtree arguments and response metadata. Count omitted items consistently with depth, archive filtering and the 300-item cap; retain existing `includeArchived` compatibility.

## Required before implementation

Finish a per-wake replay, including reply usefulness, event kinds, and independent and combined A/B estimates. Label a sample of actual stopped turns, then assess Jev predictions and confidence against those labels. Inspect artifact ownership and user-message delivery paths. Test queue persistence, user interruption, race handling, settings, authorization, artifact ownership and subtree filters; use a fake HTTP server for triage.
