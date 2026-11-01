# Project rules

This repository reconstructs the Windows release of Colin McRae Rally from
shipped binaries and user-owned data. It is not a distribution of the game.

## Non-negotiable boundaries

- Never commit, vendor, embed, or publish original executables, DLLs, media,
  archives, fonts, palettes, tracks, car data, or other game assets.
- Read original media in place. Generated manifests may contain paths relative
  to a user-selected source root, sizes, hashes, and format metadata, but not
  payload bytes.
- Never commit a developer's absolute original-media path. Commands and code
  must accept an explicit `--source` root or use the documented
  `CMR_ORIGINAL_DIR` example variable.
- Put local REA snapshots and investigation output under `.work/`.
- Put imported or converted user data under `runtime-data/`. Both directories
  are ignored by Git.
- Describe decompiler output as pseudocode, not original source. Separate
  observations, inferences, design decisions, and unknowns.
- Running the user's original game binaries (`Rally.exe` and its companions)
  is allowed (owner decision, 2026-11-02) for the compatibility-layer work:
  run them from a local game folder outside the repository, never commit the
  binaries or anything copied out of them, and record runtime observations in
  the evidence ledger as runtime evidence (separate from static evidence).

## Reconstruction order

1. Establish exact artifact identities and evidence provenance.
2. Recover file formats, ABI boundaries, state machines, and subsystem behavior.
3. Implement an original-compatible build against explicit compatibility
   interfaces.
4. Verify behavior with differential fixtures or controlled observations.
5. Add modern platform backends (such as SDL) without weakening the
   compatibility baseline.

Do not modernize behavior while it is still being recovered. A modern backend
must implement an already-defined project interface and pass the same relevant
contract tests.

## Decompilation and subsystem wiring rules

- **Verify x86 condition codes and signedness against disassembly**: Never trust
  decompiler pseudocode for byte/word comparisons (`< 0`, `< 0x80`), shifts, or
  widening multiplies without checking the raw x86 instructions (`jl`/`jg` vs
  `jb`/`ja`, `imul` vs `mul`, `movsx` vs `movzx`). A single signed-vs-unsigned
  branch error (e.g., `cmp al, 0x80` followed by `jl` vs `jb` in
  `FUN_0040aa40`) can silently disable an entire physics fallback path.
- **Reconstruct full routines rather than stopping at the first consumer**: When
  a routine updates both simulation state and visual/presentation state (such as
  `FUN_00416760` updating `roll_state_88` for tyre loads and `visual_roll_state_130`
  for Node 5 body roll/heave), reconstruct the complete address range or
  explicitly document the unimplemented tail in `docs/DECOMP_MAP.md`.
- **Trace the caller's live execution order and data flow end-to-end**: Unit
  testing a helper in isolation with synthetic defaults is not sufficient.
  Always verify in the parent orchestrator (e.g., `FUN_00407690`,
  `FUN_00409b40`, `FUN_00439ee0`, `FUN_004a7c90`):
  1. Every struct field read by a helper is populated on the live path and
     initialized with the binary's exact setup constants (e.g., `FUN_00407220` /
     `FUN_00407270`).
  2. Loops and sub-steps execute in the exact binary call order (e.g., wheel
     velocity/slip preparation before tyre-load application).
  3. Recomputed state is propagated across dependent subsystems (e.g., syncing
     damage-scaled gear ratios into the active gearbox state, posing parent
     scene-graph nodes such as Node 5 `[car + 0x04]` in addition to leaf wheel
     nodes, and running sub-tick render interpolation alongside the 25 Hz
     simulation tick).

## Code and commits

- Prefer Rust for new code when it does not obstruct binary-compatible layouts,
  calling conventions, or faithful translation. Isolate unavoidable C/C++ or
  assembly behind narrow interfaces.
- Keep parsers pure and bounds-checked. Preserve unknown fields and reject
  malformed or truncated input with useful offsets.
- Tests and fixtures must be source-owned and redistributable. Tiny synthetic
  fixtures may reproduce a format shape; do not derive fixtures by copying game
  payloads.
- Recover behavior in test-backed increments. Every completed parser rule,
  state transition, or compatibility contract needs focused unit coverage,
  including meaningful malformed and boundary cases, before its milestone
  commit. Run the full existing suite to prevent regression before advancing.
- Express platform/backend expectations as shared contract tests so the later
  modern backend is checked against the same behavior as the compatibility
  implementation.
- Make milestone commits only after the relevant checks pass. Use Conventional
  Commit subjects.
- Update the evidence ledger and roadmap in the same milestone that changes a
  compatibility claim.
- Update `docs/DECOMP_MAP.md` and recompute its final progress-tail counters in
  every milestone that completes or materially advances a reconstruction item.
  Keep the progress block as the final lines so a developer can monitor it with
  `tail`/`watch`.

