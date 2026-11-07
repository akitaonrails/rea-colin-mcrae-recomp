# Changelog

All notable user-visible changes to this project are documented here.

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
