# Bughunt2 subagent rules — read first

Caveman mode: terse, fragments OK, drop articles/hedging. Output markdown English.

## Hard rules
- No shortcuts. No "later". No spot-fix. Systemic only.
- File:line mandatory: `crates/foo/src/bar.rs:142`.
- Verify in source. No guessing.
- Do NOT edit files. Report only.
- Return ≤8 bullets to main thread after writing report.

## Hunt categories (universal)
1. unwrap/expect/panic! in hot paths
2. unsafe without SAFETY: comment
3. data races / Arc<Mutex> misuse / lock-order / poisoned recovery
4. resource leaks (GPU buffer/texture, file handle, channel)
5. integer overflow / lossy casts (`as u32` from usize/u64, narrowing)
6. TODO/FIXME/HACK/XXX unfinished
7. dead code (private items never called)
8. DRY violations / duplicated logic
9. error swallowing: `let _ =`, `.ok()`, `.unwrap_or_default()`, `.map_err(|_|...)`
10. heavy `.clone()` in hot loops
11. allocation in hot loops (`Vec::new`, `format!`)
12. logic bugs: off-by-one, wrong unit, inverted bool, axis swap
13. f32 NaN/inf (div, sqrt, log)
14. async/threading: blocking on UI thread, Mutex across `.await`, channel deadlock
15. lifetime hacks: 'static where shorter fits, unneeded Box<dyn>

## PT-specific extras (T1/T3)
- PDF zero-check, MIS weights, balance heuristic
- RNG per-thread state collision
- ray epsilon (self-intersection)
- BRDF eval at roughness=0, IOR=0
- light sampling: zero-prob, NEE occluder check
- tonemap / colour-space confusion (linear vs sRGB)
- ONB build, tangent-space math
- Russian roulette bias

## GPU-specific (T2)
- wgpu/vulkan usage flags wrong, missing labels
- shader binding setup copypasta
- buffer creation boilerplate dedup
- format mismatch (RG8 vs RG16 etc)

## App-specific (T4)
- UI thread blocking
- file I/O missing error handling
- egui antipatterns (state in render, missing id_source)
- Windows paths (backslash vs forward, UNC, case)
- Drop order / channel close races

## Output format → `.bughunt/agent_tN.md`
```
# Agent TN — <scope>
Files scanned: N

## Findings
### [CRITICAL] short title
- Loc: path:LINE
- Cat: N
- Issue: terse
- Why bad: systemic
- Fix: proper

### [HIGH] ...
### [MED] ...
### [LOW] ...

## Dead code
- path:LINE name — why no callers

## Dedup
- groups of similar blocks

## Notes
- anything weird
```

## Tools
- `mcp__filesystem__grep_files` for regex sweep
- `mcp__filesystem__read_text_file` with `line_numbers=true`
- `Glob` to enumerate files
