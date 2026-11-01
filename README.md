# Colin McRae Rally (1998) — Portable Reconstruction

A clean-room, static-analysis-driven portable reconstruction of the original
1998 Windows release of **Colin McRae Rally** in safe Rust (with SDL3 for
windowing, audio, and input, plus a hardware-accelerated `wgpu` renderer and a
bit-exact software C3D reference rasterizer).

> **Note:** This repository contains **no original game assets, executables, or
> proprietary data**. To play, you must point the asset importer at your own
> legal installation of the 1998 PC release of *Colin McRae Rally* (European
> release / official EU patch baseline). All imported and extracted game data
> is stored locally inside the Git-ignored `runtime-data/` directory.

[![Gameplay preview of the native Rust reconstruction of Colin McRae Rally (1998)](https://new-uploads-akitaonrails.s3.us-east-2.amazonaws.com/2026/11/01/colin-mcrae-rally-recomp/showcase-poster.jpg)](https://new-uploads-akitaonrails.s3.us-east-2.amazonaws.com/2026/11/01/colin-mcrae-rally-recomp/showcase-h264.mp4)

*Gameplay preview of the native Rust reconstruction (`cmr-app --windowed-game-session`): front-end menus, stage loading, 3D stage & car rendering, checkpoints/split HUD, and co-driver audio. Click to watch (2¼ minutes, with sound).*

---

## Two-Branch Architecture (`main` vs. `modern`)

This repository pursues two complementary goals across two official branches
(see [`CONTRIBUTING.md`](CONTRIBUTING.md) for contributor guidelines and
[`.github/workflows/release.yml`](.github/workflows/release.yml) for automated
dual-binary builds):

| Branch | Binary | Purpose & Scope |
| --- | --- | --- |
| **`main` (Preservation)** | `cmr-app-preservation` | **1:1 Historical Preservation**: Strict, instruction-verified replication of the 1998 EU `Rally.exe` behavior, Q16.16 fixed-point physics, 8-bit paletted lighting, and `cmRally.cfg` data formats. Audited in [`docs/PRESERVATION_AUDIT.md`](docs/PRESERVATION_AUDIT.md). |
| **`modern` (Enhanced)** | `cmr-app-modern` | **Modern Enhancements (always ahead of `main`)**: Adds native 1080p / 1440p / 4K & widescreen (`16:9` / `16:10` / `21:9`) rendering, 60 / 120 / 144+ FPS continuous sub-tick interpolation, extended draw distance (`1.0x`–`3.0x`), anisotropic filtering (`4x`–`16x`), MSAA (`2x`–`8x`), FSR/CAS upscaling, in-game Modern Settings menus (`save/modern.cfg`), and modern Gamepad mappings + rumble by default. |

---

## Highlights & Current Status

- **Complete 119 / 119 Reconstruction Map (`docs/DECOMP_MAP.md`)**:
  All 119 tracked reverse-engineering and reconstruction items across file
  formats, fixed-point Q16.16 four-wheel vehicle physics, surface/weather
  dynamics, collision & damage, front-end menus, Championship & Rally School
  progression, replays, 3D rendering, and QMixer audio are implemented and
  backed by **2,910+ automated tests**.
- **Native Hardware & Software Rendering**:
  Supports hardware-accelerated rendering via `wgpu` (`--renderer auto` or
  `--renderer gpu`) alongside a pure-Rust software C3D rasterizer
  (`--renderer software`) that reproduces the original 8-bit paletted lighting,
  distance fog, horizon sky, Y-billboarded trackside foliage, planar car
  shadows, and night headlight pools.
- **Full Front-End, Championship, Single Stage, Time Trial & Rally School**:
  Navigate the rotating 3D front-end menus, configure your car in the Service
  Area, drive any of the 56 stages across 8 countries (New Zealand, Greece,
  Monaco, Australia, Sweden, Corsica, Indonesia, UK), hear Nicky Grist's
  co-driver pace notes, and save/load profiles and Championship progress.
- **Seamless Keyboard & Gamepad Support**:
  Hot-plugged SDL3 gamepad support works across all front-end menus, sub-menus,
  name entry, pause screens, and in-race driving (analog triggers/sticks and
  D-pad), alongside full keyboard controls.

---

## Quick Start (How to Play)

### 1. Prerequisites

- **Rust toolchain** (`rustup`, stable Rust 1.85+)
- **SDL3** development libraries installed on your system (or built via Cargo)
- **Your own copy of Colin McRae Rally (1998 PC, EU release)** located outside
  this repository

Point `CMR_ORIGINAL_DIR` at your original game installation directory (the
folder containing `Rally.exe`, `Game/`, `New_Fe/`, and `ini/`):

```sh
export CMR_ORIGINAL_DIR="/path/to/your/CMRally"
```

### 2. One-Time Asset & Table Import

Verify and import the game's media files and executable-backed physics/lookup
tables into your local `runtime-data/` folder (which is ignored by Git):

```sh
mkdir -p runtime-data

# 1. Verify and import game media (stages, cars, menus, audio, fonts)
cargo run --release -q -p cmr-assets -- verify \
  --source "$CMR_ORIGINAL_DIR" \
  --profile profiles/installed-eu-derived-assets-v2.json

cargo run --release -q -p cmr-assets -- import \
  --source "$CMR_ORIGINAL_DIR" \
  --profile profiles/installed-eu-derived-assets-v2.json \
  --output runtime-data/installed-eu-derived-assets-v2

# 2. Extract executable-backed physics, surface, and static lookup tables
cargo run --release -q -p cmr-assets -- extract-surface-tables \
  --source "$CMR_ORIGINAL_DIR" \
  --profile profiles/official-eu-surface-tables-v1.json \
  --output runtime-data/official-eu-surface-coefficients

cargo run --release -q -p cmr-assets -- extract-compiled-car-initializer \
  --source "$CMR_ORIGINAL_DIR" \
  --profile profiles/official-eu-compiled-car-initializer-v1.json \
  --output runtime-data/official-eu-compiled-car-initializer

cargo run --release -q -p cmr-assets -- extract-car-selectors \
  --source "$CMR_ORIGINAL_DIR" \
  --profile profiles/official-eu-car-selector-map-v1.json \
  --output runtime-data/official-eu-car-selectors

cargo run --release -q -p cmr-assets -- extract-static-tables \
  --source "$CMR_ORIGINAL_DIR" \
  --profile profiles/official-eu-static-tables-v1.json \
  --output runtime-data/official-eu-static-tables
```

### 3. Launch the Game

Always run with `--release` for full frame rate (40 Hz front-end menus and
smooth 3D stage rendering):

```sh
cargo run --release -p cmr-app -- \
  --windowed-game-session \
  --runtime-data runtime-data/installed-eu-derived-assets-v2 \
  --source "$CMR_ORIGINAL_DIR/Game/Au/1" \
  --stage AU1 \
  --car-code Sub \
  --renderer auto
```

*(Note: `--source`, `--stage`, and `--car-code` set the initial fallback stage
and car, while selecting a country/stage/car in the in-game menus dynamically
loads the chosen stage and vehicle from your runtime-data store.)*

---

## Controls

Both **Keyboard** and **Gamepad** are active at all times and can be used
interchangeably in menus and on stage.

| Action | Keyboard | Gamepad (Xbox / PlayStation / SDL) |
| --- | --- | --- |
| **Menu Up / Down / Left / Right** | Arrow Keys | D-Pad or Left Stick |
| **Menu Confirm / Select** | `Enter` | `South` (`A` / `Cross`) or `Start` |
| **Menu Back / Cancel** | `Escape` or `Backspace` | `East` (`B` / `Circle`) or `Back` |
| **Steer Left / Right** | `Left` / `Right` Arrows | Left Stick X or D-Pad Left / Right |
| **Throttle (Accelerate)** | `Up` Arrow or `A` | Right Trigger (`RT`/`R2`), Right Stick Up, or `South` (`A`/`Cross`) |
| **Brake / Reverse** | `Down` Arrow or `Z` | Left Trigger (`LT`/`L2`), Right Stick Down, or `West` (`X`/`Square`) |
| **Handbrake** | `Space` | `East` (`B` / `Circle`) |
| **Shift Up / Gear Up** | `S` | Right Shoulder (`RB` / `R1`) |
| **Shift Down / Gear Down** | `X` | Left Shoulder (`LB` / `L1`) |
| **Change Camera View** | `C` | `North` (`Y` / `Triangle`) |
| **Look Behind** | `V` | — |
| **Pause Race** | `Escape` | `Start` or `Back` / `Select` |

---

## Technical Overview & Architecture

This project reconstructs a buildable, portable implementation of the original
Windows game from static analysis. The first target is behavioral and data
compatibility with a precisely identified shipped build. Platform modernization
comes afterward, behind compatibility interfaces.

The workspace is organized into focused crates:
- [`crates/cmr-formats`](crates/cmr-formats): Pure, bounds-checked parsers for
  CMPR-compressed streams, `.c3d` 3D models, `.bm`/`.pal` bitmaps and palettes,
  `.fon` bitmap fonts, `.cat`/`.csp`/`.dat`/`.hpc`/`.srf`/`.moj`/`.anm`/`.rsf`
  stage sections, and `.ssr`/`.rsp` replay files.
- [`crates/cmr-assets`](crates/cmr-assets): SHA-256-authenticated asset
  verifier, importer, and PE static-table extractor.
- [`crates/cmr-core`](crates/cmr-core): Fixed-point Q16.16 four-wheel physics,
  tire/surface lookup, static/moving/car-vs-car OBB collision & crash damage,
  chase/bonnet/bumper/cockpit cameras, C3D lighting & UV preparation, front-end
  menu carousel state machines, Championship & Rally School logic, and replay
  seeding/stepping.
- [`crates/cmr-video`](crates/cmr-video): Software C3D rasterizer, 8-bit
  palette lighting/fog tables, menu/HUD/service-area compositors, and AVI
  video decoding.
- [`crates/cmr-gpu`](crates/cmr-gpu) & [`crates/cmr-gpu-stage`](crates/cmr-gpu-stage):
  Hardware-accelerated `wgpu` indexed-palette presenter and persistent 3D stage
  renderer.
- [`crates/cmr-audio`](crates/cmr-audio): 16-API `QSWaveMix` software mixer,
  engine RPM pitch/crossfade synthesizer, surface/skid/impact audio, and
  co-driver speech sequencer.
- [`crates/cmr-net`](crates/cmr-net): DirectPlay lobby/session protocol and
  opt-in TCP transport.
- [`crates/cmr-platform-sdl3`](crates/cmr-platform-sdl3): SDL3 windowing,
  keyboard/gamepad input translation, audio output stream, and 40 Hz / 25 Hz
  fixed-step frame pacing.
- [`crates/cmr-app`](crates/cmr-app): End-to-end application orchestrator,
  CLI, and headless verification harnesses.

### Known Reference Artifacts

| Role | Size (bytes) | SHA-256 |
| --- | ---: | --- |
| `Rally.exe` (installed EU-derived) | 1,114,190 | `46500c8e22ce3e943b6855eca63ae9288261162347ca76fa36cad77cbb19c0bb` |
| `Rally.exe` (official EU updater clean reference) | 1,114,112 | `0eb6ea9652219b9a35c0a6134bf0cdf316dce21215b724cdfc10ba85eb95982a` |
| `Game.exe` | 502,396 | `b445fd0dc6243b652a894c299a336296f48e35f7034775d4d60edd30fcbdee78` |
| `qmixer.dll` | 136,192 | `262c3247b130c9de8410f11f39898230b1776bff5acbd820125d1e06ae260b26` |

Unsupported boundaries are explicit: native driver-owned padding and exact
font/filter/alpha output, alternate type-1 lighting, a concrete FFB hardware
device, other regional editions/locales, and native Windows or GitHub Actions
execution not run locally. See [PRESERVATION_AUDIT.md](docs/PRESERVATION_AUDIT.md),
[DECOMP_MAP.md](docs/DECOMP_MAP.md), [COMPARISON.md](docs/COMPARISON.md),
[DEVIATIONS.md](docs/DEVIATIONS.md), and [CONTRIBUTING.md](CONTRIBUTING.md)
for the 1:1 surface-area audit, evidence ledger, documented boundaries, and
branch contribution rules.

The [public reconstruction survey](docs/UPSTREAM_SURVEY.md) records related
projects and the searches used to avoid duplicating existing work. The optional
PlayStation reference-edition workflow is documented in
[PSX_REFERENCE_PLAN.md](docs/PSX_REFERENCE_PLAN.md).

---

## Developer Verification & Benchmarks

All generated reverse-engineering state belongs in `.work/`, and all imported
game data belongs in `runtime-data/`. Neither is versioned.

Run all source-owned unit, integration, lint, and portability checks with:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check-portability.sh
./scripts/check-legacy-backends.sh
```

Run the offscreen indexed-palette GPU conformance slice (`wgpu` vs `SDL_GPU` vs
software oracle) with:

```sh
SDL_VIDEO_DRIVER=offscreen cargo run -q -p cmr-gpu-conformance -- \
  --backend both --warmup 2 --frames 8
```

Run the opt-in raw-C3D cross-backend hardware check:

```sh
SDL_VIDEO_DRIVER=offscreen cargo test -p cmr-gpu-conformance \
  both_backends_match_exactly_on_one_raw_recovered_c3d_packet \
  -- --ignored --nocapture
```

Run the steady-state GPU stage benchmark over an imported stage:

```sh
SDL_VIDEO_DRIVER=offscreen cargo run --release -q -p cmr-gpu-conformance \
  --bin cmr-gpu-stage-benchmark -- \
  --source "$CMR_ORIGINAL_DIR/Game/Au/1" --stage AU1 \
  --warmup 32 --frames 240
```

Render one complete deterministic root-menu frame headlessly:

```sh
cargo run -q -p cmr-app -- \
  --headless-root-frame \
  --runtime-data runtime-data/installed-eu-derived-assets-v2
```

Run the opt-in reviewed-import root-session smoke test:

```sh
CMR_RUNTIME_DATA="$PWD/runtime-data/installed-eu-derived-assets-v2" \
  cargo test --locked -p cmr-video --test reviewed_import_session_smoke \
  -- --ignored --nocapture
```

### Documentation Index

- [docs/ROADMAP.md](docs/ROADMAP.md) — Execution plan and milestone history
- [docs/EVIDENCE.md](docs/EVIDENCE.md) — Static and runtime evidence ledger
- [docs/DECOMP_MAP.md](docs/DECOMP_MAP.md) — Live 119-item checklist and progress footer (`watch -n 2 'tail -n 10 docs/DECOMP_MAP.md'`)
- [docs/SUBSYSTEMS.md](docs/SUBSYSTEMS.md) — Recovered subsystem boundaries
- [docs/ANALYSIS.md](docs/ANALYSIS.md) — Reproducible reverse-engineering workflow
- [docs/ASSET_PIPELINE.md](docs/ASSET_PIPELINE.md) — Asset verification and import boundary
- [docs/COMPARISON.md](docs/COMPARISON.md) — Differential verification methodology
- [docs/DEVIATIONS.md](docs/DEVIATIONS.md) — Supported editions and intentional differences
- [docs/MODERN_BACKEND_RESEARCH.md](docs/MODERN_BACKEND_RESEARCH.md) — GPU/platform backend evaluation

---

## Legal Note

“Abandonware” is not a legal license. This repository contains independently
written source code, documentation, and synthetic tests only. Users remain
responsible for obtaining and using the original game data lawfully.
