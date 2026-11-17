# Refract Parley patch

Vendored from crates.io parley 0.12.0 (Apache-2.0 OR MIT; upstream licenses retained).
The only source change is `PlainEditor::set_ranged_styles` and its backing storage
in `src/editing/editor.rs`. This lets the host supply CRDT-owned formatting while
retaining Parley caret layout, selection, hit testing and IME editing. Spans use
UTF-8 byte offsets and must be refreshed by the host after edits. Remove this
patch once upstream offers equivalent externally supplied editor style ranges.
