# Plan JSON

Publish a UTF-8 `.json` file with `artifact_publish`, `kind: "plan"` and a title. See
[plan-example.json](plan-example.json) for all sections. Arrays retain display order. Prose fields
(`summary`, `goal`, `text`, `claim`, `change`) allow inline markdown (bold, code, links); everything
else is plain text. Unknown keys are rejected. Summary chips (counts by ticket status, open
questions) are computed by Mast. Ids are nonempty, unique within their array, and stable across
revisions.

The top-level object has `kind`, `title`, `summary`, and optional `sheet`, `scope`, `findings`,
`decisions`, `tickets`, `contracts`, `questions`, `blastRadius`, `verification` and `superseded`.

- `sheet`: `{ "ref": "mast-artifact:<id>", "rev": 14 }`, the decision sheet this plan builds on.
  Ticket `behavior[].row` ids are checked against it.
- `scope`: `constraints[]` (strings), `in[]` (strings), `out[]` (`item`, `livesIn`: a ticket id,
  `"later"` or `"dropped"`), `optional[]` (`item`, `question`: a question id).
- `findings[]`: `id`, `claim`, `cite` (`{ "path": "file", "line": 96 }` or `{ "url": "https://…" }`,
  or null for a claim with no source), `status` (`measured`, `claimed`, `inferred`, `unverified`),
  optional `section` for grouping (`current`, `evidence`). `measured` and `claimed` require a
  `cite`. Mast renders `cite` as a small source chip; `status` colours its dot.
- `decisions[]`: `id`, `title`, `text` (decided in words), `rows[]` (sheet row ids), `origin`.
- `tickets[]`, in build order:
  - `id` (`T1`, `T2`, …; the number is the build order), `title`, `goal`, optional `linear`
    (`{ "id": "ENG-1234", "url": … }`, null until step 7 creates the ticket).
  - `behavior[]`: `text` and one of `row` (a sheet row id), `rows[]` (several), or `questions[]` (the behavior waits on an open question). `acceptance[]`: strings.
    `outOfScope[]`: `text`, `livesIn` (ticket id, `later`, `dropped`, or a question id).
  - `contract`: `{ "ref": <contract id>, "role": "produces" | "consumes" }` or null.
  - `where[]`: `{ "path", "line", "status" }` (`measured` by default).
  - `dependsOn[]`: `{ "id": "T1", "mode": "build" | "merge" }` (`merge`: can start now, merge after),
    plus optional `external[]`: `{ "label", "url", "state": "open" | "merged" }`.
  - `status`: `{ "state": "buildable" | "blocked-by-tickets" | "blocked-by-questions", "questions": [ids] }`.
  - `size`: `{ "label": "S" | "M" | "L", "lines": [low, high] }`. Lines are a diff estimate, not time.
- `contracts[]`: `id`, `name`, `summary`, `producedBy[]` and `consumedBy[]` (ticket ids), optional `example`.
- `questions[]`: `id`, `text`, `row` (a sheet row id), `blocking`, `blocks[]` (ticket ids), `example`.
  A plan with a sheet may omit this array and let the sheet carry the questions.
- `blastRadius[]`: `surface`, `kind` (`page`, `endpoint`, `model`, `task`), `audience`, `change`, `gate`.
- `verification[]`: `ticket` (id or null for plan-wide), `text`. `superseded[]`: `item`, `reason`, `date`.

## Validation

Publish refuses a plan that fails any of these, with the path of the first offender:

1. A ticket's `dependsOn[].id` must be a lower ticket id that exists. `T1` depends on nothing.
2. `status.state` must match the data: `blocked-by-tickets` needs a `build` dependency;
   `blocked-by-questions` needs `questions[]` that exist and are `blocking`; `buildable` has neither.
3. Every `behavior[].row`/`rows[]` (and `decisions[].rows[]`) must exist on the linked `sheet`, and its
   status there must be `decided`. An open row can only be referenced through `questions[]`.
4. Every question's `blocks[]` ticket lists that question in its `status.questions`.
5. `contract.ref` must name a contract that lists the ticket in `producedBy` or `consumedBy`.
6. No time fields: keys such as `days`, `hours`, `eta`, `estimate` and `duration` are rejected, as are
   `size.lines` values that aren't integers with `low ≤ high`.
7. `measured` and `claimed` findings and `where[]` entries need a `cite`/path.
8. A ticket whose `goal`, `behavior` or `acceptance` is empty is rejected; `outOfScope` and `where` may be empty.

## Linear body

The renderer builds each ticket's Linear body from its fields: `# <title>`, then Goal, Behavior,
Acceptance, Out of scope, Contract, Where, Depends on and Size, each as plain markdown with row ids
in brackets. Step 7 of the planning skill copies that body into Linear verbatim and writes the new
issue id back into `linear`.
