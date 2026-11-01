# Contributing to Colin McRae Rally (1998) Recompilation

Thank you for contributing! This repository pursues **two complementary goals** across two official branches:

1. **`main` — 1:1 Preservation Branch**:
   Strict, instruction-verified 1:1 behavioral and data replication of the original 1998 Windows release (`Rally.exe`, European patched baseline) for historical preservation and reverse-engineering accuracy.
2. **`modern` — Modern Enhancements Branch**:
   Built directly on top of `main` (**always kept ahead of `main`**), adding native modern features: 1080p/1440p/4K widescreen resolutions, 60/120/144+ FPS frame rates with continuous sub-tick pose interpolation, extended draw distance, anisotropic texture filtering, MSAA, FSR upscaling, in-game Modern Graphics/Display settings screens, and modern plug-and-play Gamepad support with rumble.

---

## 1. Branch Model & Where to Open Pull Requests

```text
main   (1:1 Preservation Baseline — strict 1998 Rally.exe fidelity)
  \
   +---> modern   (Modern Enhancements — always rebased/merged ahead of main)
```

### Target `main` when your change is:
- Fixing a physics, collision, animation, audio, UI, or file-format discrepancy against the original `Rally.exe` disassembly.
- Promoting a **Category B** (partial/simplified) or **Category C** (unimplemented legacy mode such as `.gho` ghost cars, `.SSR` live replay recording, secondary `.c3d` car child nodes, or 2-player split-screen) routine in [`docs/PRESERVATION_AUDIT.md`](docs/PRESERVATION_AUDIT.md) to **Category A** (verified 1:1 instruction-faithful).
- Improving asset verification, static-table extraction, portability, or deterministic tests that apply equally to both branches.

### Target `modern` when your change is:
- Adding or improving high-resolution rendering (`1080p`, `1440p`, `4K`, Steam Deck `1280x800`), widescreen/ultrawide aspect-ratio frustum expansion, or HUD scaling.
- Adding or improving high-FPS (`60`/`120`/`144`/Uncapped) continuous sub-tick interpolation, VSync, extended draw distance, anisotropic filtering, MSAA, or FSR/CAS upscaling.
- Enhancing the in-game Modern Settings menus (`Options -> Graphics / Display / Controller`) or `save/modern.cfg` persistence.
- Improving modern Gamepad auto-detection, button prompts (`A`/`B`/`X`/`Y`/`LB`/`RB`/`LT`/`RT`), or SDL3 gamepad rumble.

### Keeping `modern` Ahead of `main`
Every commit landed on `main` must be merged (or rebased) forward into `modern` so `modern` is **strictly ahead of `main`**:

```sh
git checkout modern
git merge main
# Run full checks on modern, then push:
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git push origin modern
```

---

## 2. Non-Negotiable Project Rules (Both Branches)

From [`AGENTS.md`](AGENTS.md):

1. **Zero Proprietary Assets in Git**:
   Never commit, vendor, embed, or publish original executables (`Rally.exe`, `Game.exe`, `qmixer.dll`), media archives, tracks, cars, fonts, palettes, or audio files. All tests must use small synthetic fixtures that test format structure without copying game payloads.
2. **No Hardcoded User Paths**:
   Never commit a developer's absolute original-media path. Always accept `--source` or `CMR_ORIGINAL_DIR` and keep Reverse Engineering Assistant (REA) outputs under `.work/` and imported assets under `runtime-data/`.
3. **Verify x86 Disassembly on `main`**:
   - Never trust decompiler pseudocode for byte/word comparisons (`< 0`, `< 0x80`), shifts, or widening multiplies without checking raw x86 instructions (`jl`/`jg` vs `jb`/`ja`, `imul` vs `mul`, `movsx` vs `movzx`).
   - Reconstruct complete routines rather than stopping at the first consumer, or document any unimplemented tail in [`docs/DECOMP_MAP.md`](docs/DECOMP_MAP.md) and [`docs/PRESERVATION_AUDIT.md`](docs/PRESERVATION_AUDIT.md).
   - Trace the parent orchestrator (`FUN_0043afd0`, `FUN_00407690`, `FUN_00439ee0`, `FUN_004a7c90`) end-to-end to verify live initialization constants, execution order, and cross-subsystem state propagation.

---

## 3. Required Pre-Commit Checks

Before opening a PR or pushing a milestone commit on either branch, run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check-portability.sh
./scripts/check-legacy-backends.sh
```

Use Conventional Commit subjects (e.g., `fix(core): ...`, `feat(modern): ...`, `docs(audit): ...`). When completing or advancing a preservation item on `main`, update [`docs/DECOMP_MAP.md`](docs/DECOMP_MAP.md) (including its `PROGRESS-TAIL` footer) and [`docs/PRESERVATION_AUDIT.md`](docs/PRESERVATION_AUDIT.md).

---

## 4. GitHub Actions CI & Modern Binary Automation

- **Continuous Integration ([`.github/workflows/ci.yml`](.github/workflows/ci.yml))**:
  Runs automatically on pushes and pull requests to both `main` and `modern`, verifying formatting, portability gates, Windows cross-compilation (`x86_64-pc-windows-gnu`), and full-workspace tests with bundled SDL3 on Linux and Windows.
- **Modern Build & Release ([`.github/workflows/release.yml`](.github/workflows/release.yml))**:
  Builds the exact triggering `modern` commit as a Linux x86-64 AppImage and a
  Windows x86-64 ZIP. Branch pushes produce inspected CI artifacts; a verified
  `v*` tag publishes those exact-tag artifacts and `SHA256SUMS.txt` to GitHub
  Releases. The preservation branch remains buildable from source and is not
  silently substituted into a modern release.

See [`docs/RELEASING.md`](docs/RELEASING.md) for artifact contents, release
gates, and the tag procedure.
