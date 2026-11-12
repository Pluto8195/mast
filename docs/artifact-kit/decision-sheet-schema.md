# Decision sheet JSON

Publish a UTF-8 `.json` file with `artifact_publish`, `kind: "decision-sheet"` and a title. See
[decision-sheet-example.json](decision-sheet-example.json) for all sections. Arrays retain display
order; strings are plain text. Unknown keys are rejected. Summary chips are computed by Mast.

The top-level object has `kind`, `title` and optional arrays `flows`, `models`, `endpoints`, `rules`,
`validations`, `permissions`, `ui_states`, and `out_of_scope`. Every object in those arrays and
all nested row objects need a nonempty `id`, unique throughout the sheet and stable across
revisions. Every top-level decision row (including each rules row) has `status: {"state": "open" | "decided" |
"rejected", "date": ...}`. Open rows use a null date; decided and rejected rows require an ISO
calendar date (`YYYY-MM-DD`). Model fields, endpoint lines and callers, flow steps and parallel steps may omit status and inherit it from their parent. Top-level decision rows may include optional `recommendation` and `question` strings. Summary counts include top-level decision rows only, never their children. Keep IDs when republishing; do not reuse an ID for a different row.

- Models have `name`, `fields` (`name`, `marker`, `type`, `null`, `default`, `constraint`).
- Endpoints have `name`, `request` and `response` lines (`marker`, `text`), and optional `callers` (`name`, `effect`).
- Rules have `columns` and `rows` (`state`, optional `action`, `cells`); cells are `✓`, `—`, or `?`, one per column.
- Validations have `rule`, `enforced` (strings), and `result`. Permissions have `subject` and
  `access` (subject-to-access string map).
- Flows have `title` and `steps` (`kind`, `label`, `params`); kinds are `ui`, `api`, `db`, `event`,
  `effect`, and `none`. A step may include `parallel` steps with the same fields (except `parallel`) to show a fan.
- UI states have `label` and `image` (a `mast-artifact:` reference or null); references render as links to the screenshot; out-of-scope rows have `label`.

Field markers are `+`, `~`, `−`, or empty; endpoint markers are `+`, `−`, or empty. Nullable
`default`, `constraint`, and `image` are strings or null.


Each top-level decision row may set `optional: true` (default `false`). Use it only when the
change can be omitted without leaving the plan incomplete. Only optional rows offer **Don't do
this**. Non-optional rows offer **Accept** and **Answer differently**.

For planners reading `artifact_get_decisions`: `outcome: decided` with
`overrideRecommendation: false` accepts the recommendation; its note is supplemental. With
`overrideRecommendation: true`, `answer` contains the user's required decision text, also stored
as the note; follow that answer instead of the recommendation. `outcome: rejected` means the
optional change in that row will not be built; its note is optional context. Preserve row IDs
and apply these decisions when revising the plan.
