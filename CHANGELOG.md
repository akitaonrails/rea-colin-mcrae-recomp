# Changelog

All notable user-visible changes to this project are documented here.

## [0.1.4] - 2026-11-20

### Fixed

- `cmr-assets`: when the parent directory of `--output` does not exist (or is
  not a real directory), all 11 extract/import commands now report
  `output parent is not a real directory: <path>` instead of misleadingly
  calling it `source is not a real directory` (#2).
- `cmr-app-modern --help` / `-h` now prints usage and exits 0 instead of
  failing with `unknown argument "--help"` (#2).
- Bundled `docs/HD_ART.md` into the Windows release ZIP and added a
  self-contained "Quick Start (From a Release ZIP)" section to `README.md`
  covering the clean EU v1.5 `Rally.exe` requirement, all 11 table extracts,
  retry cleanup (`output already exists`), and the HD pack path (#2, #1).

## [0.1.3] - 2026-11-19

### Added

- Periodic 50-tick mid-stage replay seek keyframes (`FUN_0046c7d0` /
  `FUN_0046ca70`) and interactive State-8 post-stage replay controls (`P`
  pause/resume, `[` / `]` ±2 s seek, `Home` restart).
- Time-Trial ghost car recording, `.cmrg` persistence, and 4-tick interpolated
  translucent `.c3d` car-mesh playback (`TimeTrialGhostRecording` /
  `SnapshotCarLane` / `FUN_0046c7d0` / `FUN_0046c910` / `FUN_0046ca70`).
- Classic analog dial HUD with rotated tachometer needle (`640x480a/b/c.bmp` +
  `needle.bmp`, `FUN_0045feb0` / `FUN_00460c40`) for Classic and Group-B cars.
- 3D barrier/scenery/tumble collision sparks (`FUN_0041a900` / `FUN_0041ab00`).
- Heavy-storm lightning flashes (`FUN_0048cec0` / `FUN_0048cf20`) and
  cockpit-vs-exterior `rain_in.wav` interior rain loop switching
  (`FUN_0048d8e0` / `FUN_0048b680`).
- Decaying force-feedback dual-motor rumble (`FfbRumbleMixer` / `FUN_00433750`
  / `FUN_00433ac0` / `FUN_00433ec0`) wired into SDL3 gamepad rumble.
- Progressive car-body mud/dust accumulation & glass-safe lower-body tinting
  (`FUN_00458670` / `FUN_00459260`) and Jelly-Car vertex wobble cheat
  (`FUN_0041d790`, `live_cfg[0x26] & 1`).

### Fixed

- Resolved Rust 1.88.0 `clippy::nonminimal_bool` CI warning in `cmr-app`.
- Completed full State-5 disassembly audit and deterministic function-map
  classification (`verified_1to1=54`, `wired_live=690`, `partial=0`).

## [0.1.2] - 2026-11-19

### Added

- In-race pacenote corner arrows (`images/osdnew/arrws.bmp`, `FUN_004625d0`) with
  2-slot sliding animation, horizontal left/right mirroring, 12-waypoint TTL
  dismissal, and `ARROWS` option gating.
- Falling rain streaks and `IMAGES/SNOW.BMP` snow weather particles
  (`FUN_0048b960..FUN_0048c620`) with wind slewing, periodic intensity
  variation, camera-box wrapping, and night attenuation.
- Clear-day sun, halo, and 7-ring lens flare plus clear-night moon and 60
  twinkling stars (`IMAGES/LENSFLR/*`, `FUN_00482d10..FUN_00485100`).
- `GAME DISPLAYS` OFF / MINIMAL / FULL race HUD modes (`FUN_0045f530` /
  `FUN_00464930`).
- Deterministic `Rally.exe` `.text` function completeness map
  (`docs/function-map.json`) and call-graph verifier
  (`scripts/check-function-map.py`).

### Fixed

- Post-stage State-8 replay now plays back and loops the recorded State-5 run
  instead of showing a static frame (`FUN_0046d8f0`, `FUN_0046dd50`,
  `FUN_00438100`).
- State-4 stage loading screen now advances its `BAR.BMP` progress bar
  monotonically over the target stage's `.LOA` background (`FUN_00421da0`).
- Smooth sub-tick frame pacing (`alpha_q16 = 0` on sim-tick frames), wired
  `ModernSettings::vsync` to `SDL_SetRenderVSync`, and sticky gamepad/keyboard
  driving device mode so neutral sticks no longer flip back to keyboard mid-turn.

## [0.1.1] - 2026-11-07

### Fixed

- The Windows ZIP now bundles `vcruntime140.dll`, the Microsoft Visual C++
  runtime that both programs and `SDL3.dll` import. A clean Windows 10 or 11
  installation does not include it, so the previous ZIP could fail to start
  with "VCRUNTIME140.dll was not found". Microsoft's redistributable licence
  allows bundling it.

## [0.1.0] - 2026-11-01

### Added

- Complete playable reconstruction of the reviewed European PC release,
  including front-end flows, championship and rally-school progression,
  four-wheel simulation, collision and damage, audio, replays, and saving.
- Native SDL3 Linux and Windows hosts with software and hardware-accelerated
  rendering backends.
- Modern branch options for high-resolution widescreen output, high frame
  rates, interpolation, MSAA, texture filtering, draw distance, upscaling,
  gamepads, and rumble.
- Opt-in content-addressed HD art-pack support without redistributing original
  game data.
- Data-authentication and import tools for a user-owned original installation.
- Linux x86-64 AppImage and Windows x86-64 ZIP release automation.

### Fixed

- HiDPI window sizing and live classification-HUD car thumbnails.
- Cross-platform GPU selection now considers Vulkan, Direct3D 12, and OpenGL,
  checks adapter capabilities before selection, and retries compatible adapters
  on hybrid/multi-GPU systems before the existing software fallback.
- Cross-platform source-file validation and line-ending stability on Windows.

[Unreleased]: https://github.com/akitaonrails/rea-colin-mcrae-recomp/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/akitaonrails/rea-colin-mcrae-recomp/releases/tag/v0.1.0
