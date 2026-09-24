# squarebob-core

squarebob-core holds shared domain types and helpers used by multiple crates in the app.

## Why this exists
The app, renderer, and PT layers all need to operate on the same file-tree model. Keeping those
structures in a small core crate avoids dependency cycles and reduces duplication.

## What it provides
- `DirEntry` tree node (path, sizes, counts, children, layout rectangle, and optional LoD expansion metadata).
- Iterative tree traversal and mutation helpers, plus size sorting.
- `LodKind` and `LodExpandInfo` for collapsed file groups.

## Where it is used
- `crates/treemap`: layout + coloring.
- `crates/render-3d` and `crates/render-shared`: rendering and picking.
- `src/app`: owned scan tree, UI state, and filtering.

`DirEntry::rect` uses `Cell` so layout can update rectangles without cloning the tree. Persistence uses an explicit flat representation; `DirEntry` has no recursive serde implementation.
