# Headless UI driver worlds

`small` is the original fixture. `medium` is a fixed synthetic world; use it
in a scenario with `world: { preset: medium, seed: 1 }`. Scenario runs remain
report-only in the release script.

`scale` requires a private JSON shape. Create a SQLite backup first; the
extractor opens only the backup and emits aggregate counts. It also reads local
Git metadata to count repositories, worktrees and branches. Keep the resulting
shape outside this public repository.

Driver engine startup requires the sandbox and `MAST_DISABLE_EXTERNAL_IMPORT=1`.
The switch disables external-session import and its background repair jobs;
the sandbox also denies reads of the account's live stores.

```sh
mkdir -p /private/tmp/ui-driver-shape
sqlite3 -readonly "$HOME/.zeron/profiles/local/docs.sqlite3" '.backup /private/tmp/ui-driver-shape/docs.sqlite3'
MAST_SHAPE_SOURCE=/private/tmp/ui-driver-shape/docs.sqlite3 \
  MAST_SHAPE_OUTPUT="$SHAPE_FILE" \
  cargo test -p mast-ui-driver --test extract_shape -- --ignored --exact extract_shape_from_backup
```

Supply the shape with `world.shape` in a scenario or with
`MAST_UI_DRIVER_SHAPE`. For example, use `world: { preset: scale, seed: 1 }`
and run `MAST_UI_DRIVER_SHAPE="$SHAPE_FILE" scripts/ui-drive run "$SCENARIO"`.
Set both variables to private local paths first. Scale scenarios are excluded from release
runs. The driver copies the counts into its isolated scratch directory before
starting; it does not grant the sandbox access to the source file.
