<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **dirstat-rs** (2651 symbols, 6575 relationships, 230 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/dirstat-rs/context` | Codebase overview, check index freshness |
| `gitnexus://repo/dirstat-rs/clusters` | All functional areas |
| `gitnexus://repo/dirstat-rs/processes` | All execution flows |
| `gitnexus://repo/dirstat-rs/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->


# HANDOFF — dirstat-rs cross-session resume

**Branch:** `main`
**Author/operator:** ssoj13

## Documentation map (cross-session)

- **`CHANGELOG.md`** — What shipped each sprint.

## Build / test commands

```powershell
# Release build (the user runs this; can't build while .exe is running)
cargo build --release --message-format=short 2>&1 | grep -E "error|warning:" | head -10

# Clippy on the path tracer crate
cargo clippy -p pt-megakernel --all-targets --message-format=short 2>&1 | grep -E "error|warning:" | head -10

# Tests
cargo test -p pt-megakernel -p pt-wavefront --message-format=short 2>&1 | grep -E "test result|FAILED" | head -10

# Profile-style run with PT logging
.\target\release\dirstat-rs.exe --log-modules pt 2>&1 | Tee-Object profile.log | Select-String "upload_scene|WF dispatch|cache MISS|scene_upload|bvh_build"
```

## User context / collaboration patterns

- **Language:** RU in chat, EN in code/comments/commits. Operator
  prefers terse responses, no fluff.
- **Decision-making:** prefers honest pushback over agreeable
  half-solutions. If a refactor is too large for one session, SAY
  SO and commit a checkpoint rather than risk broken state.
- **Frustration triggers:** flapping behaviour (commit clamp, revert
  clamp, re-add clamp — operator called this "снапить блять"); long
  speculative responses without concrete progress.
- **Verification style:** operator runs the release exe themselves,
  shows screenshots, points at visual artifacts. They diagnose the
  USER side; we diagnose the CODE side.
- **Snapping/UI:** WF Tile is clamped {0} ∪ [64, 8192] in the UI AND
  host. 0 = full frame (no tiling). Drag-down uses halfway split
  (<32 → 0, 32..63 → 64) so the user can drag to "off" via mouse.

## Things to NOT do

- Don't add new features speculatively. Operator wants concrete
  progress on known bugs / Stage G plan.
- Don't blindly re-run `npx gitnexus analyze` — last session it
  segfaulted. The "GitNexus index stale" warning from hooks can be
  ignored unless explicitly asked.
- Don't auto-rebuild release while operator might be running the
  exe — it'll fail to write the .exe. Wait for explicit "rebuild"
  signal or just cargo check first.
- Don't undo / partially-undo recently shipped commits. Operator
  asked twice in a row "don't snap" then "add clamp back" — go with
  the LATER instruction.
