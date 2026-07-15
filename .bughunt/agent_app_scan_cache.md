# Agent app_scan_cache — Scan, cache, and UI orchestration

Files scanned: 14

Scope: `src/scanner.rs`, `src/scanner_ntfs.rs`, `src/cache.rs`, `src/path_key.rs`, `src/app/scan_orchestration.rs`, `src/app/mod.rs`, `src/app/state.rs`, `src/app/tree_panel.rs`, `src/app/status_bar.rs`, `src/app/settings/scanner.rs`; related callers in `src/app/toolbar.rs`, `src/app/render_loop.rs`, `src/app/screenshot.rs`, `src/exclusions.rs`.

## Findings

### [HIGH] Starting another scan orphans active worker

- Loc: `src/app/toolbar.rs:28`
- Loc: `src/app/toolbar.rs:38`
- Loc: `src/app/toolbar.rs:47`
- Loc: `src/app/toolbar.rs:63`
- Loc: `src/app/scan_orchestration.rs:86`
- Loc: `src/app/scan_orchestration.rs:103`
- Loc: `src/app/scan_orchestration.rs:120`
- Loc: `src/scanner.rs:46`
- Loc: `src/scanner.rs:157`
- Loc: `src/scanner_ntfs.rs:114`
- Loc: `src/scanner_ntfs.rs:128`
- Cat: 4, 9, 14
- Issue: History selection, Enter, and folder picker remain active during scan. Each calls `start_scan`. `start_scan` replaces `scan_rx` and `scan_cancel` without cancelling previous session. Dropped receiver disconnects old sender, but both scanners ignore send failures and continue disk walk/MFT work.
- Why bad: Repeated path changes create concurrent full scans. Old workers consume CPU, I/O, and memory. App loses cancellation handle. Thread creation also panics on failure at `src/scanner.rs:53` and `src/scanner_ntfs.rs:146`.
- Fix: One `ScanSession` owner: generation ID, root identity, cancel token, receiver, worker handle. Every scan entry point calls one replacement transition. Transition cancels active generation, retires receiver, reaps completed handles without blocking UI, then starts next generation. Scanner sends must stop work on disconnected receiver. Spawn returns recoverable error.
- Impact: GitNexus downstream `start_scan` = CRITICAL, 28 symbols, 6 processes, 5 modules. `scan_ntfs_bg` = HIGH. No edit permitted without approval.

### [HIGH] NTFS cancellation becomes fallback or cacheable partial success

- Loc: `src/scanner_ntfs.rs:121`
- Loc: `src/scanner_ntfs.rs:130`
- Loc: `src/scanner_ntfs.rs:134`
- Loc: `src/scanner_ntfs.rs:367`
- Loc: `src/scanner_ntfs.rs:817`
- Loc: `src/scanner_ntfs.rs:864`
- Loc: `src/scanner_ntfs.rs:896`
- Loc: `src/scanner_ntfs.rs:921`
- Loc: `src/scanner_ntfs.rs:929`
- Loc: `src/scanner_ntfs.rs:835`
- Cat: 9, 12, 14
- Issue: `scan_ntfs_bg` treats every `scan_mft_usn` error, including explicit cancellation, as NTFS failure and starts jwalk fallback with already-set cancel flag. Later cancellation is worse: `build_subtree` breaks and returns partial tree; `fill_sizes` returns `()`, checks only each 5,000th file, and cannot propagate cancellation. `build_tree_from_mft` still returns `Ok(tree)`; caller sends `Done`; UI serializes cache.
- Why bad: Stop can show false NTFS-fallback error or commit incomplete tree as successful scan. Cancellation semantics depend on phase.
- Fix: Typed terminal outcome: `Completed`, `Cancelled`, `Failed`. Recursive build and size passes return `Result`/control flow and propagate cancellation immediately. Fallback only for classified NTFS capability/runtime failures. Never send `Done` or cache after cancellation.
- Impact: GitNexus downstream `scan_ntfs_bg`, `scan_mft_usn`, and `build_tree_from_mft` = HIGH. No edit permitted without approval.

### [HIGH] NTFS volume selection accepts unrelated first letter

- Loc: `src/scanner_ntfs.rs:29`
- Loc: `src/scanner_ntfs.rs:77`
- Loc: `src/scanner_ntfs.rs:323`
- Loc: `src/scanner_ntfs.rs:466`
- Loc: `src/scanner_ntfs.rs:643`
- Loc: `src/scanner_ntfs.rs:773`
- Loc: `src/scanner_ntfs.rs:784`
- Cat: 8, 12
- Issue: Six paths derive drive by first ASCII alphabetic character anywhere in string. Relative `folder` becomes volume `F:`; UNC `\\server\share` becomes `S:`. Availability probe, raw handle, diagnostics, dump, and subtree navigation can disagree with actual root.
- Why bad: NTFS mode can probe/open wrong local volume, enumerate unrelated MFT, then return empty/error tree for requested root. UNC and relative paths lack reliable fallback classification.
- Fix: One Windows `VolumeTarget` parser. Normalize input to absolute path. Accept only explicit disk/verbatim-disk prefixes supported by raw scanner. Route UNC, relative-resolution failure, unsupported prefixes, mount points, and cross-volume aliases to standard scanner with explicit reason. Reuse same helper in probe, availability, scan, diagnostics, dump, and tree scoping.
- Impact: GitNexus downstream `scan_mft_usn` = HIGH; `scan_ntfs_bg` = HIGH. No edit permitted without approval.

### [HIGH] Cache miss and invalid path retain stale derived display state

- Loc: `src/app/scan_orchestration.rs:47`
- Loc: `src/app/scan_orchestration.rs:80`
- Loc: `src/app/scan_orchestration.rs:86`
- Loc: `src/app/mod.rs:362`
- Loc: `src/app/mod.rs:364`
- Loc: `src/app/tree_panel.rs:112`
- Loc: `src/app/state.rs:262`
- Loc: `src/app/state.rs:305`
- Cat: 12
- Issue: Cache-miss branch clears `tree`, `filtered_tree`, `cache_age` only. It leaves `display_tree_cache`, extension stats, size/filter bounds, expanded paths, and filtered-path cache. Invalid-path return clears nothing. `display_root` prioritizes old `display_tree_cache` whenever free-space display or newly loaded exclusions are active.
- Why bad: New path can render and expose previous path tree while scan runs or after invalid-path error. Stats and filters also describe previous root.
- Fix: Atomic presentation-state transition keyed by scan generation/root identity. Clear every derived tree/stat/filter/selection/render field together when no same-root preview exists. Keep cached preview only when envelope root matches active normalized identity. Use same install/reset path for cache preview, live completion, cancellation, and failure.
- Impact: GitNexus downstream `start_scan` = CRITICAL; `display_root` = HIGH, 4 affected processes. No edit permitted without approval.

### [HIGH] Cached preview arms automated screenshot before live scan

- Loc: `src/app/mod.rs:192`
- Loc: `src/app/scan_orchestration.rs:76`
- Loc: `src/app/scan_orchestration.rs:78`
- Loc: `src/app/scan_orchestration.rs:189`
- Loc: `src/app/screenshot.rs:21`
- Loc: `src/app/screenshot.rs:25`
- Loc: `src/app/screenshot.rs:32`
- Loc: `src/app/screenshot.rs:57`
- Cat: 12
- Issue: Startup comment says screenshot timer starts after scan completion. Cache-load branch starts timer before live scanner starts. Completion branch cannot reset it because field is already `Some`.
- Why bad: Delay can expire during rescan. Capture uses stale cached tree, marks screenshot taken, and may exit process before live result.
- Fix: Tie capture readiness to live scan generation and successful terminal completion. Cache remains preview only. If cached capture is desired, expose explicit policy; never infer readiness from preview availability.
- Impact: Owning `start_scan` downstream impact = CRITICAL. No edit permitted without approval.

### [HIGH] Full cache work blocks UI thread

- Loc: `src/app/scan_orchestration.rs:60`
- Loc: `src/app/scan_orchestration.rs:63`
- Loc: `src/app/scan_orchestration.rs:73`
- Loc: `src/cache.rs:103`
- Loc: `src/cache.rs:112`
- Loc: `src/app/scan_orchestration.rs:159`
- Loc: `src/app/scan_orchestration.rs:167`
- Loc: `src/app/scan_orchestration.rs:169`
- Loc: `src/app/scan_orchestration.rs:215`
- Cat: 14
- Issue: UI callback opens and deserializes entire cache, computes recursive stats/range, rebuilds display tree, and serializes full live tree before spawning writer.
- Why bad: Multi-million-entry trees can freeze event loop on scan start and completion. Moving disk write only leaves dominant serialization and derivation work on UI.
- Fix: Background pipeline owns `DirEntry` while loading/deriving/serializing, then sends owned tree plus derived metadata/cache bytes to UI. No `Arc<DirEntry>`; preserve main-side ownership after channel handoff. Install result in one bounded UI transition. Coalesce progress, bound pending messages.
- Impact: GitNexus downstream `start_scan` = CRITICAL; `poll_scan` = HIGH. No edit permitted without approval.

### [MED] Detached, non-atomic cache writes race rescan and clear

- Loc: `src/app/scan_orchestration.rs:171`
- Loc: `src/cache.rs:89`
- Loc: `src/app/settings/scanner.rs:43`
- Loc: `src/cache.rs:112`
- Loc: `src/cache.rs:131`
- Cat: 4, 9, 14
- Issue: Each completion spawns untracked writer. `fs::write` truncates final file in place. Same-root completions can finish out of order. Immediate rescan can read partial file. Clear can delete before older writer recreates file. Corrupt-cache removal failure is discarded.
- Why bad: Cache can regress to older snapshot, disappear, reappear after user clears it, or be repeatedly unreadable. Valid envelope lacks requested-root validation before install.
- Fix: Single cache service with normalized-key queue and monotonically ordered generation. Write same-directory temp file, flush, atomically replace. Clear becomes ordered tombstone invalidating older writes. Load uses bounded decoding, validates version/root identity/tree invariants, reports removal/quarantine failure. Surface persistent cache errors.
- Impact: GitNexus `write_cache_bytes` = LOW locally; owning `poll_scan` = HIGH across orchestration. Systemic change still crosses HIGH process.

### [HIGH] Scanner errors become zero-byte, cacheable success

- Loc: `src/scanner.rs:94`
- Loc: `src/scanner.rs:124`
- Loc: `src/scanner.rs:125`
- Loc: `src/scanner.rs:137`
- Loc: `src/scanner.rs:225`
- Loc: `src/scanner_ntfs.rs:287`
- Loc: `src/scanner_ntfs.rs:296`
- Loc: `src/scanner_ntfs.rs:429`
- Loc: `src/scanner_ntfs.rs:857`
- Loc: `src/scanner_ntfs.rs:904`
- Loc: `src/scanner_ntfs.rs:915`
- Cat: 9, 12
- Issue: Standard scanner continues every walk error, including possible root failure, and maps metadata error to size zero without incrementing error count. NTFS parser skips malformed records at trace level, reports zero errors, silently truncates depth beyond 256, and maps metadata failure to zero size. Both can send `Done`; UI caches result.
- Why bad: Disk-usage total silently undercounts. Zero-byte value is indistinguishable from real empty file. Cache preserves incomplete result.
- Fix: Shared `ScanDiagnostics` and completeness contract. Root failure terminal. Recoverable child failures counted and classified. Metadata failure represented as unknown/omitted with diagnostic, never fabricated zero. Depth/cycle guard returns explicit incomplete/error status. Cache envelope records completeness; UI labels partial results and policy controls whether partial snapshots are reusable.
- Impact: GitNexus downstream `scan_dir` = HIGH; `scan_mft_usn` = HIGH. No edit permitted without approval.

### [MED] NTFS directory count includes root; progress contract diverges

- Loc: `src/scanner_ntfs.rs:830`
- Loc: `src/scanner_ntfs.rs:833`
- Loc: `src/scanner_ntfs.rs:927`
- Loc: `src/scanner_ntfs.rs:931`
- Loc: `src/scanner_ntfs.rs:915`
- Loc: `src/scanner_ntfs.rs:918`
- Loc: `src/scanner.rs:202`
- Cat: 8, 12
- Issue: `fill_sizes` increments `dc` for root, then overwrites root `dir_count` with `dc`. Standard aggregation counts child directories only. NTFS progress always sends `bytes: 0` and `errors: 0`, even during metadata pass.
- Why bad: Backend switch changes directory total by one. Status contract differs by engine.
- Fix: One post-order aggregate definition shared by both engines: root counts descendants, not itself. Carry cumulative bytes and diagnostics through progress. Derive final counters from tree once; do not overwrite with differently defined traversal totals.
- Impact: GitNexus downstream `build_tree_from_mft` = HIGH despite local `fill_sizes` = LOW. No edit permitted without approval.

### [MED] Raw path strings split one filesystem root into multiple identities

- Loc: `src/path_key.rs:6`
- Loc: `src/path_key.rs:8`
- Loc: `src/cache.rs:33`
- Loc: `src/exclusions.rs:25`
- Loc: `src/exclusions.rs:33`
- Loc: `src/exclusions.rs:62`
- Loc: `src/app/scan_orchestration.rs:54`
- Loc: `src/app/scan_orchestration.rs:55`
- Cat: 8, 12
- Issue: Cache/exclusion key hashes raw UTF-8 input. Exclusion membership and history dedupe also compare raw strings. Windows case, separator, relative/absolute, verbatim, trailing-separator, and symlink/junction aliases produce different identities for same root.
- Why bad: Cache and exclusions appear missing across equivalent spellings. History duplicates roots. State can diverge permanently.
- Fix: Central `ScanRoot` value: display path, normalized absolute operational path, stable identity key. OS-aware normalization; retain UNC semantics; avoid lossy conversion for identity. Cache, exclusions, history, scan generation, and envelope validation consume same key. Migrate old raw-key files on successful validated load.
- Impact: GitNexus reports `scan_path_id_hex` LOW and misses textual caller edges; filesystem cross-check confirms both `src/cache.rs:34` and `src/exclusions.rs:63`. Owning `start_scan` remains CRITICAL.

## Dead code

- None proposed.
- Non-Windows API-parity stubs at `src/scanner_ntfs.rs:69`, `src/scanner_ntfs.rs:625`, `src/scanner_ntfs.rs:751`, and `src/scanner_ntfs.rs:960` intentionally preserved.
- No TODO/FIXME/HACK/XXX marker found in scoped files.

## Dedup

- Drive extraction duplicated at `src/scanner_ntfs.rs:29`, `src/scanner_ntfs.rs:77`, `src/scanner_ntfs.rs:323`, `src/scanner_ntfs.rs:466`, `src/scanner_ntfs.rs:643`, `src/scanner_ntfs.rs:773`. Replace class with one volume-target parser.
- Cache-preview install and live-result install duplicate stats/range/tree/layout/screenshot transitions at `src/app/scan_orchestration.rs:60` and `src/app/scan_orchestration.rs:151`. One source-aware result installation transition.
- Standard and NTFS metadata/error/progress policies diverge at `src/scanner.rs:124` and `src/scanner_ntfs.rs:896`. One scanner result/diagnostics contract.
- Cache and exclusions already share `src/path_key.rs:6`; evolve this single identity source instead of adding another hash helper.
- Scanner thread shells duplicate spawn/cancel/sort/terminal-send behavior at `src/scanner.rs:32` and `src/scanner_ntfs.rs:106`. One session runner around backend-specific scan body.

## Impact audit

- `start_scan`: CRITICAL — 28 downstream symbols, 6 processes, 5 modules.
- `poll_scan`: HIGH — 11 downstream symbols, 3 processes, 2 modules.
- `scan_ntfs_bg`: HIGH — 14 downstream symbols, 3 processes, 3 modules.
- `scan_mft_usn`: HIGH — 11 downstream symbols, 3 processes, 3 modules.
- `build_tree_from_mft`: HIGH — 5 downstream symbols, 3 processes, 2 modules.
- `display_root`: HIGH — 3 downstream symbols, 4 processes, 2 modules.
- `scan_dir`: HIGH — 2 downstream symbols, 3 processes.
- `load_cache`, `write_cache_bytes`, `serialize_cache`, `scan_path_id_hex`, `fill_sizes`: local LOW; owning orchestration reaches HIGH/CRITICAL.
- Approval required before edits touching HIGH/CRITICAL symbols.

## Notes

- `DirEntry` ownership remains main UI side after channel delivery. Proposed workers own trees only before handoff. No shared `Arc<DirEntry>`.
- GitNexus primary index worked. Secondary `gitnexus_rs` endpoint rejected lbug v42 with storage v40; not used for conclusions.
- No builds/tests run. Bughunt source-analysis phase only.
- No source code, AGENTS.md, or DIAGRAMS.md changed.
