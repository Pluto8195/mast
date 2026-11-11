# Workflow pack format (work in progress)

The initial proposed format is a UTF-8 JSON object with `format` equal to
`mast.workflow-pack/v1`, `mode` equal to `full` or `share`, and arrays named
`scopes` and `templates`. It is not yet exposed by Mast and is subject to
change until export and import ship together.

Each scope has `itemPath`, an array of item titles from the Space root, and
`instructions`, an ordered array of strings. An empty path means the Space
root. Scope paths are unique after trimming and case folding each title. A
template has `name` and `body`; the body uses the frozen project-template
tree, containing titles, descriptions, project and worker-destination flags,
instructions, and children. Template names are unique after trimming and
case folding. Registry ids, chat assignments, and revision history are not
part of this representation.

This format does not yet implement redaction, file size or content limits,
child templates, settings, or skills. In particular, choosing `share` does not
itself make content safe to share. No file should be exported or imported
using these types until the full validation, preview and undo paths exist.
