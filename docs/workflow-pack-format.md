# Workflow pack

A workflow pack carries a Space's Work Outline setup (instructions and project
templates) to another Space. Settings → Workflow pack exports and imports it;
admin chats use the `work_outline_workflow_pack_*` tools.

## File format

A UTF-8 JSON object, at most 4 MiB, with `format` equal to
`mast.workflow-pack/v1`, `mode` equal to `full` or `share`, and arrays named
`scopes` and `templates`.

Each scope has `itemPath`, an array of item titles from the Space root, and
`instructions`, an ordered array of strings. An empty path means the Space
root. Scope paths are unique after trimming and case folding each title. A
template has `name` and `body`; the body uses the frozen project-template
tree, containing titles, descriptions, project and worker-destination flags,
instructions, and children. Template names are unique after trimming and
case folding. Registry ids, chat assignments, and revision history are not
part of the file.

Export redacts credentials in both modes, and in `share` mode also email
addresses and home folder paths. Detection is pattern-based and incomplete:
read a pack before you share it. Child templates, settings, and skills are not
part of the format.

## Import

1. **Preview** reads the file and compares it with the destination. It changes
   nothing and returns a preview id. Scopes are matched by item path and
   templates by name.
2. **Apply** takes the preview id and, for every conflict (a scope or template
   that exists with different content), a resolution: `skip` keeps the
   destination's, `append` adds the instructions it lacks, `replace` archives
   the destination's instructions or swaps the template body. A conflict
   without a resolution is refused. Items the pack needs are created. Apply
   runs under one registry lock: every guard is checked and every inverse
   dry-run first, and a failure rolls back what was written.
3. **Undo** replays the stored receipt's inverses, and refuses without
   changing anything if an imported instruction, item or template was edited,
   used, or moved since.

Receipts are host-local JSON files in `workflow-pack-receipts/` under the
profile data directory, one per apply, so undo works after a restart.

## RPC and tools

RPC: `WorkflowPackExportPreview`, `WorkflowPackImportPreview`,
`WorkflowPackApply`, `WorkflowPackUndo`, `WorkflowPackReceipts`. None is
forwarded to another device.

MCP tools (`work_outline_workflow_pack_export`, `_preview_import`, `_apply`,
`_undo`, `_receipts`) work only in an admin chat, on the granted Space.
