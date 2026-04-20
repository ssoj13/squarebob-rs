# dirstat-core

dirstat-core holds shared domain types and helpers used by multiple crates in the app.

## Why this exists
The app, renderer, and PT layers all need to operate on the same file-tree model. Keeping those
structures in a small core crate avoids dependency cycles and reduces duplication.

## What it provides
- `DirEntry` tree node (path, sizes, counts, rect, children).
- Shared helpers/types used by treemap and render pipelines.

## Where it is used
- `crates/treemap`: layout + coloring.
- `crates/render-3d` and `crates/render-shared`: rendering and picking.
- `src/app`: UI state and filtering.
