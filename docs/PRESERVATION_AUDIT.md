# `Rally.exe` 1:1 Preservation & Fidelity Audit

This document provides the instruction-level surface-area and fidelity audit of the canonical 1998 European release of **Colin McRae Rally** (`Rally.exe`) against this repository's Rust reconstruction on the `main` (1:1 Preservation) branch.

- **Canonical Clean Reference Binary**: `Rally.exe` (official EU updater payload, `1,114,112` bytes, `SHA-256: 0eb6ea9652219b9a35c0a6134bf0cdf316dce21215b724cdfc10ba85eb95982a`)
- **Installed EU-Derived Comparison Binary**: `Rally.exe` (`1,114,190` bytes, `SHA-256: 46500c8e22ce3e943b6855eca63ae9288261162347ca76fa36cad77cbb19c0bb`)
- **Scope**: All `2,168` procedures carved in Ghidra (`B01` in [`docs/DECOMP_MAP.md`](DECOMP_MAP.md)), covering `EXTERNAL` imports and `.text` (`0x401000..0x4e5b00`).

---

## 1. Binary Surface-Area Breakdown (`2,168` Procedures)

### 1.1 Reconciling Total Procedure Counts

| Metric | Procedure Count | Address Range | Description |
| --- | ---: | --- | --- |
| **Total Ghidra Procedures (`B01`)** | **2,168** | `EXTERNAL` + `0x401000..0x4e5b00` | `166` `EXTERNAL` PE import symbols + `2,002` procedures in `.text` (`1,995` internal functions + `7` thunks). |
| **Total `.text` Procedures** | **2,002** | `0x401000..0x4e5b00` | Every procedure in `.text`. |
| **Less: Embedded MSVC 6.0 CRT** | **-350** | `0x4cfab0..0x4e5b00` | `srand` (`0x4cfab0`), `rand` (`0x4cfac0`), `__ftol` (`0x4cfb9c`), `qsort`/`shortsort` (`0x4d01b0`), `WinMainCRTStartup` (`0x4d1800`), `__alldiv` (`0x4d1ec0`), `__nh_malloc` (`0x4d2540`), `stdio`/FP formatting/Win32 OS handle wrappers (`0x4d3000..0x4e5b00`). |
| **Less: Embedded `zlib 1.1.3`** | **-16** | `0x4ccdc0..0x4cf8e0` | `inflate`, `inflate_blocks`, `inflate_codes`, `inflate_trees`, `inflate_fast`, `adler32`, `zcalloc`, `zcfree`. |
| **Codemasters `.text` Procedures** | **1,636** | `0x401000..0x4ccd60` | All Codemasters-compiled procedures before `zlib` and the MSVC 6.0 CRT. |
| **Less: PE Import & WinMM `avivideo` Thunks** | **-14** | `0x4af8a0..0x4afe5f` (9) + `0x4b8760..0x4b880e` (5) | `5` single-instruction `jmp` import thunks (`DPLAYX::Ordinal_1/2`, `DDRAW::DirectDrawCreate/EnumerateA`, `DINPUT::DirectInputCreateA`) + `9` WinMM `mciSendCommandA` AVI window wrappers. |
| **Net Codemasters Application Functions** | **1,622** | `0x401000..0x4ccd60` | The exact **1,622** non-CRT, non-zlib, non-thunk application functions. |

### 1.2 Address-Range Layout of `.text`

In `Rally.exe`, the compiler is **MSVC 6.0** and the embedded C runtime starts at `0x4cfab0` (immediately following embedded `zlib 1.1.3` at `0x4ccdc0..0x4cf8e0`):

| Address Range | Total Procs | Subsystem Breakdown | Preservation Classification |
| --- | ---: | --- | --- |
| **`0x401000..0x4af36f`** | **1,346** | **1,178** Core Game / Physics / Collision / Scene Graph / Track / Audio / HUD / Replay / School functions + **168** Win32 / DirectDraw4 / D3D2 / QMixer / DirectInput / DirectPlay / Registry OS wrappers | **1,178 Core Game** + **168 Platform/API Wrappers** (legitimately replaced by Rust `std` + `cmr-platform-sdl3` + `wgpu`) |
| **`0x4af370..0x4ccdbf`** | **290** | `12` UI helpers (`0x4af370..0x4af89f`) + `9` MCI AVI wrappers (`0x4af8a0..0x4afe5f`) + `42` D3D2 texture/palette/fog/LVERTEX functions (`0x4afe60..0x4b376f`) + `39` Championship C3D/3D setup functions (`0x4b3770..0x4b875f`) + `5` PE import thunks (`0x4b8760..0x4b881f`) + `183` `New_Fe` Frontend menus, options (`FUN_004ca520..0x4cadb0`), `cmRally.cfg` (`FUN_004b9e70`/`FUN_004ba5e0`), and main loop `FUN_004c1e00` (`0x4b8820..0x4ccdbf`) | **235 Core Frontend / Championship / Render-Batch Logic** + **55 Platform/D3D2/MCI/Import Thunks** |
| **`0x4ccdc0..0x4cf8e0`** | **16** | Embedded `zlib 1.1.3` (`inflate*`, `adler32`, `zcalloc`, `zcfree`) | **100% Replaced** by bounded `flate2` decompression in `crates/cmr-formats` |
| **`0x4cfab0..0x4e5b00`** | **350** | MSVC 6.0 CRT (`srand`, `rand`, `__ftol`, `qsort`, `entry`, `__alldiv`, `stdio`, Win32 heap/handle wrappers) | **100% Replaced** by Rust `std` (with exact `GameLcg` `rand` and `__ftol` truncation preserved where simulation determinism depends on them) |
| **`EXTERNAL` Imports** | **166** | `KERNEL32` (83), `USER32` (38), `GDI32` (11), `ADVAPI32` (6), `WINMM` (9), `QMIXER` (14), `DDRAW` (2), `DINPUT` (1), `DPLAYX` (2) | **100% Replaced** by `cmr-platform-sdl3`, `wgpu`, `cmr-audio`, and Rust `std` |
| **TOTAL** | **2,168** | **1,413 Core Game/Sim/Render/UI** + **223 Platform/DirectX/Audio/Input/Net Wrappers** + **366 CRT/zlib** + **166 External Imports** | — |

---

## 2. True 1:1 Fidelity Classification of the `1,413` Core Game Functions

Auditing the x86 disassembly of `Rally.exe` against `crates/cmr-core`, `crates/cmr-formats`, `crates/cmr-video`, `crates/cmr-audio`, and `crates/cmr-app` divides the **1,413 pure game/engine/UI functions** (`1,178` in `0x401000..0x4af36f` + `235` in `0x4af370..0x4ccdbf`) into three preservation categories:

| Category | `0x401000..0x4af36f` | `0x4af370..0x4ccdbf` | Total Core Functions | Share of All Core (`1,413`) | Share of 1P Core (`1,163`) | Description |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| **Category A: Verified 1:1 Instruction-Faithful** | **646** | **148** | **794** | **56.2%** | **68.3%** | Full routine logic, Q16.16 fixed-point arithmetic (`imul`+`shrd 16`), signed/unsigned x86 condition codes, constants, and live caller data flow match `Rally.exe`. |
| **Category B: Implemented with Partial / Simplified Tails or Host-Rendered Equivalents** | **310** | **59** | **369** | **26.1%** | **31.7%** | Implemented and unit-tested (`119/119` items in `DECOMP_MAP.md`), but either: (1) a secondary branch/tail in the binary was omitted, (2) a secondary `.c3d` child node animation is unposed, or (3) a clean-room host compositor replaces raw D3D2/DDraw state calls. |
| **Category C: Non-Core / Legacy Modes** | **222** | **28** | **250** | **17.7%** | — | 2-player split-screen (`DAT_00550c08 == 2`), LAN/DirectPlay multiplayer (`0x459400..0x45b8f0`), Time-Trial `.gho` ghost car (`0x424ae0..0x428850`), live player `.SSR` replay recording, `+0x25 & 0x80` cheat physics, and unused diagnostic stubs. |
| **Total Core Game Functions** | **1,178** | **235** | **1,413** | **100.0%** | **100.0%** | Plus `223` OS/DirectX wrappers + `366` CRT/zlib + `166` PE imports = `2,168`. |

---

## 3. Lessons Learned: Common Decompilation Pitfalls & How We Fixed Them

During the physics, handling, and animation verification passes, we discovered four classes of subtle errors that occur when trusting decompiler pseudocode or testing helpers in isolation without verifying the raw x86 instructions and live caller data flow:

### 3.1 Signed vs. Unsigned x86 Condition Codes (`jb`/`ja` vs. `jl`/`jg`)
- **What went wrong**: In `FUN_0040aa40` (force-pass wheel slip/surface lookup) and `0x40f5e0..0x40f5e8` (`select_second_wheel_candidate`), Ghidra's decompiler rendered byte/dword comparisons or candidate selections ambiguously. In `drivetrain::select_second_wheel_candidate`, replacing the proportional static-friction candidate (`normalization.length`) whenever `transition_length >= 0` instead of comparing against the static-friction ceiling `alternate` (`gain_35c`) forced the tires into full sliding friction (`gain_36c`) at any non-zero speed, causing excessive ice-like sliding and weak braking.
- **How we fixed it**: Checked the raw x86 disassembly (`0x40f5e0..0x40f5e8` and `cmp al, 0x80` followed by `jb` vs `jl`), restoring `select_signed_force_candidate(transition_length, alternate, transition)` so static grip holds up to `gain_35c` and transitions to sliding friction `gain_36c` only when exceeded.

### 3.2 Reconstructing Full Routines Instead of Stopping at the First Consumer
- **What went wrong**: `FUN_00416760` (`0x416760..0x417ce7`) has two halves:
  1. `0x416760..0x4171ec`: Computes `roll_state_88` (`ctl + 0x88`) and `corner_roll_c8` (`ctl + 0xc8..+0xd4`) for `FUN_00417cf0` tyre normal loads, and writes rotated corner vectors `[±hx, 0, ±hz]` into `ctl + 0x94..+0xc0`.
  2. `0x4171ee..0x417ca7`: Computes `visual_roll_state_130` (`ctl + 0x130`) with `2x` lateral impulse (`shl eax, 1` at `0x4173c0`), suspension heave `visual_heave_y`, and poses `[ctl + 0x00]` (C3D **Node 5**, the car body shell).
  Initially, only the first half was reconstructed for tyre loads, leaving Node 5 unposed so the car body looked stiff and unresponsive to track bumps.
- **How we fixed it**: Reconstructed the complete `0x416760..0x417ce7` address range plus `FUN_00415df0` (`0x415df0..0x416754`, lagged visual chassis follower `[ctl + 0x04]` and `-0x4ccc` ground-clearance corner check) and wired Node 5 (`BODY_NODE_ID = 5`) into `WheelRig`.

### 3.3 Fixed-Point Multiply (`imul` + `shrd 16`) vs. Division (`qdiv`) and Setup Initializer Offsets
- **What went wrong**: In `FUN_00415df0` (`0x415f3f`), the blend factor multiplies `over` by `ctl.up_blend_slope_100` (`0x50_0000` = `80.0`) via `imul` + `shrd 16` (`q(over, up_blend_slope_100)`), and tests `roll_corners_94[i].y + visual_heave_y` (where `ctl + 0xc4` is initialized at `0x4048c2` from `[ctl + 0x00]->translation.y = 0`, not `-half_height_30`). Using `-half_height_30` (`-0.685 m`) caused every corner to fail the `-0x4ccc` (`-0.30 m`) ground-clearance check at rest, lifting the car body `0.385 m` above the tires.
- **How we fixed it**: Verified `0x4048c2..0x4048cc` (`FUN_00404440`), `0x417150..0x4171ec` (`FUN_00416760`), and `0x416659..0x4166a4` (`FUN_00415df0`) in disassembly, storing `roll_corners_94` and testing `corner.y + visual_heave_y < min_corner_y_ec`.

### 3.4 Tracing Live Orchestrator Call Order and Sub-Tick Render Interpolation
- **What went wrong**: `Rally.exe` steps simulation mechanics (`FUN_0043afd0`) at a fixed **25 Hz** (`40 ms`), but renders (`FUN_004a7c90` -> `FUN_00439ee0`) on every display frame by interpolating between the previous and current 25 Hz poses via `FUN_00435400(prev, curr, alpha_q16)`. Rendering only at 25 Hz made the car animation feel low-FPS even when the physics were exact.
- **How we fixed it**: Reconstructed `interpolate_matrix_435400`, `interpolate_wheel_animation_state`, `moving_object_node_locals_interpolated` (`FUN_00430480` + `FUN_0045c9c0`), and sub-tick frame rendering in `cmr-app` / `cmr-platform-sdl3`.

---

## 4. Call-by-Call Audit of Core Live Orchestrators

### 4.1 `FUN_0043afd0` (`0x43afd0..0x43b91f`, 25 Hz Mechanics Orchestrator)

| # | Binary Address & Instruction | Target Routine & Role | Live Rust Status (`GameplaySession` / `cmr-app`) | Classification |
| ---: | --- | --- | --- | --- |
| 1 | `0x43b02a..0x43b051: call 0x41a7a0` | Copies 3 car0 node matrices when `DAT_00551af8 != 0` | `prev_tick_pose` captured before `session.advance()` | **Category A** |
| 2 | `0x43b06a: call 0x46d8f0` | Replay per-tick input recording | Offline `vertical_path.rs` only | **Category C** |
| 3 | `0x43b06f: call 0x46dd50` | Replay per-tick input playback (`-> FUN_0046dc20`) | `step_active_school_replay` runs on school demo states | **Category B** |
| 4 | `0x43b0d1: call 0x443da0(car)` | Pre-race start sequence countdown gate | `start_sequence_pre_drive` / `start_sequence_after_drive` | **Category A** |
| 5 | `0x43b0e9: call 0x41b270(ctl, car, idx)` | Steering, throttle, brake ramp & lateral velocity prep | `step_with_all_four_wheels` (`four_wheel_step.rs`) | **Category A** |
| 6 | `0x43b126..0x43b148: call 0x41b110` | 2-player control preparation | Single-player only | **Category C** |
| 7 | `0x43b183..0x43b21c` (inline) | Snapshots car/chassis/body/wheel state into prev sub-tick buffers | `prev_tick_pose` snapshot in `advance_game_frame` | **Category A** |
| 8 | `0x43b232: call 0x436b90` | Snapshots dynamic `.obj` matrices before physics | `MovingObjects::update` snapshots `prev_locals` | **Category A** |
| 9 | `0x43b256: call 0x407690(ctl, car, order, n)` | Per-car physics, drivetrain, body roll (`0x416760`), tyre loads (`0x417cf0`), forces (`0x4095e0`), integration (`0x409b40`) | `four_wheel_drive` -> `step_with_all_four_wheels` | **Category A** |
| 10 | `0x43b26b..0x43b28f: call 0x459db0/0x45a7f0` | Network multiplayer state sync & dead-reckoning | Protocol in `cmr-net`; not active in 1P | **Category C** |
| 11 | `0x43b294: call 0x43c410` | Rebuckets last tick's moved `.obj` instances into cells | `objects.rebucket(&parsed.grid)` before static collision | **Category A** |
| 12 | `0x43b29e: call 0x417f60(car0)` | Rebuilds car0 OBB/hull axes & corners before static collision | Top of `resolve_static_collision_after_step` | **Category A** |
| 13 | `0x43b2bd: call 0x43d5d0(car0)` | Static scenery collision (`FUN_0043d5d0`) | `resolve_static_collision_after_step` | **Category A** |
| 14 | `0x43b2c7: call 0x43f790(car0)` | Dynamic `.obj` moving-object collision (`FUN_0043f790`) | `resolve_moving_object_collision_after_step` | **Category A** |
| 15 | `0x43b2fa..0x43b316: call 0x442740` | Car-vs-car OBB collision (`FUN_00442740`) | Reconstructed in `car_vs_car_collision.rs` (`RaceState`) | **Category B/C** |
| 16 | `0x43b327: call 0x403a00(car0, 0)` | `.dat` trigger volume walk & barrier walls (`FUN_00403a00`) | `four_wheel_tail` -> `run_trigger_walk` | **Category A** |
| 17 | `0x43b342: call 0x418120(car0 + 0xfc)` | Rebuilds car0 world corner probes (`+0xfc`) & copies to `+0x15c` after trigger/barrier impulse | `run_trigger_walk` rebuilds `ground.probes` & `trigger_prior_15c` after `walk_triggers` | **Category A** |
| 18 | `0x43b381: call 0x450d40` | Dynamic `.obj` per-tick physics update | `objects.update` in `four_wheel_tail` | **Category A** |
| 19 | `0x43b388: call 0x45c1b0(0)` | `.moj` kinematic scenery per-tick motion | `step_moj_slot_45c1b0` in `four_wheel_tail` | **Category A** |
| 20 | `0x43b3a4..0x43b4d4` (inline) | Per-wheel render spin (`DAT_00828de0`) integration | `animate_wheel_spin` (`spin_step`) | **Category A** |
| 21 | `0x43b503: call 0x417e20(ctl, car)` | Per-wheel wobble phase (`+0x604`) & offset (`+0x5e4`) | `animate_wheel_wobble` (`wobble_step`) | **Category A** |
| 22 | `0x43b50a: call 0x410a60(ctl, car)` | Ground raycast pass, tumbling (`0x4115d0`), and `0x410350` suspension compression/travel/vibration | `run_ground_pass` + `animate_suspension` | **Category A** |
| 23 | `0x43b511: call 0x415df0(ctl, car)` | Lagged visual chassis basis (`[ctl +0x04]`) & corner clearance lift (`-0x4ccc`) | `step_visual_chassis_415df0` in `animate_suspension` | **Category A** |
| 24 | `0x43b517: call 0x467210(car, idx)` | Car reset / off-track recovery dispatcher | `run_reset` in `four_wheel_tail` | **Category A** |
| 25 | `0x43b537: call 0x47a900` | Time-Trial ghost tick (`0x428320`) & `+0x25 & 0x80` special car tick (`0x484510`) | Ghost/cheat mode not active | **Category C** |
| 26 | `0x43b5de..0x43b821` | Captures post-tick car/wheel/`.obj` targets; snaps `prev = curr` on reset | Handled in `advance_game_frame` (`SubtickRenderSnapshot`) | **Category A** |
| 27 | `0x43b846..0x43b860: call 0x4343a0(0)` | Camera controller 25 Hz step (`FUN_004343a0`) & reset snap | `camera.update` / `camera.snap` in `advance_game_frame` | **Category A** |
| 28 | `0x43b8c0..0x43b8d8: call 0x462ac0(car)` | `.cat` node-cursor progress & split/finish tracker | `progress.update` in `try_advance` | **Category A** |
| 29 | `0x43b8e2: call 0x433750(0)` | DirectInput force-feedback per-tick effect update | Reconstructed in `force_feedback.rs`; host device optional | **Category B** |

### 4.2 `FUN_00439ee0` (`0x439ee0..0x43af16`, Sub-Tick Render Pose & Scene-Graph Evaluator)

| # | Binary Address Range | Target Routine & Role | Live Rust Status (`cmr-app` / `cmr-core`) | Classification |
| ---: | --- | --- | --- | --- |
| 1 | `0x43a021..0x43a061` | Interpolates car root transform (`FUN_00435400`) at `alpha_q16` | `interpolate_matrix_435400(&prev_car_world, &curr_car_world, alpha_q16)` | **Category A** |
| 2 | `0x43a086..0x43a200` | Interpolates lagged visual chassis (`[ctl +0x04]`), Node 5 body roll (`[ctl +0x00]`), heave, bounce, and clearance lift into Node 5 | `WheelRig::locals_interpolated` -> `body_pose` + `interpolate_matrix_435400` | **Category A** |
| 3 | `0x43a22e..0x43a4b4` | Poses suspension wishbones / axles / struts (`[car +0x64]` child nodes) | Secondary `.c3d` child nodes remain at rest matrices | **Category B** |
| 4 | `0x43a57f..0x43a754` | Poses the 4 wheel nodes (`[15, 19, 13, 18]`): compression, eased, bounce, wobble, spin, camber, and steer yaw | `interpolate_wheel_animation_state` + `wheel_pose` | **Category A** |
| 5 | `0x43a770..0x43ad08` | Poses interior steering wheel, driver head/arms, mudflaps (`+0x26 & 2 == 0`), spare wheel, and cockpit gearstick/gauges | Secondary `.c3d` child nodes remain at rest matrices | **Category B** |
| 6 | `0x43adae..0x43adf9` | Sub-tick camera matrix interpolation (`FUN_00434d40`) | `interpolate_matrix_435400(&prev_camera_world, &curr_camera_world, alpha_q16)` | **Category A** |
| 7 | `0x43ae01..0x43ae11` | Sub-tick `.moj` kinematic hierarchy interpolation (`FUN_0045c9c0`) | `moving_object_node_locals_interpolated(alpha_q16)` | **Category A** |
| 8 | `0x43ae16..0x43ae21` | Sub-tick dynamic `.obj` moving-object matrix interpolation (`FUN_00430480`) | `MovingObjects::node_locals_interpolated(alpha_q16)` | **Category A** |
| 9 | `0x43ae32` | Sub-tick under-car shadow quad interpolation (`FUN_0042aba0`) | `plan_shadow` with `car_world_offset` | **Category A** |
| 10 | `0x43ae3c..0x43ae43` | Special `+0x25 & 0x80` car animation interpolation (`FUN_00484950`) | Cheat/special mode not active | **Category C** |

---

## 5. Preservation Backlog for `main` Contributors

While all `119 / 119` items in [`docs/DECOMP_MAP.md`](DECOMP_MAP.md) are complete and verified for playable single-player stages, Rally School, Time Trial, and Championship progression, contributors working on the `main` (1:1 Preservation) branch can help promote the remaining **Category B** and **Category C** routines to **Category A**:

1. **Secondary `.c3d` Car Child-Node Posing in `FUN_00439ee0` (`0x43a210..0x43ad08`)**:
   - Reconstruct the `[car + 0x64]` node-pointer bindings (`FUN_00404440`) and pose the suspension wishbones/struts (`0x43a22e..0x43a4b4`), interior steering wheel & driver head/arms (`0x43a770..0x43a965`), mudflaps (`0x43a965..0x43aa38`), spare wheel (`0x43aa39..0x43ac05`), and cockpit gearstick/gauges (`0x43ac05..0x43ad08`).
2. **Per-Car Physics Helpers `FUN_00415d20` & `FUN_004102e0` inside `FUN_00407690` (`0x407988`, `0x4079e4`)**:
   - Reconstruct `FUN_00415d20` (`ctl + 0x70..+0x84` rolling surface plane normal/height average) and `FUN_004102e0` (`ctl + 0x420` airborne/grounded transition counter) and verify all downstream consumers.
3. **Full 3x3 World Inertia Tensor Similarity Transform in `FUN_00409f90` (`0x40a2a0..0x40a470`)**:
   - Audit `car + 0x218..+0x254` (`I_world = R * I_body * R^T`) across normal driving and tumbling transitions.
4. **Time-Trial Ghost Car `.GHO` Recording & Playback (`0x424ae0..0x428850`, `FUN_00428320`)**:
   - Wire per-tick ghost capture, `.gho` persistence, and translucent ghost car rendering in Time Trial mode.
5. **Live Player Race Replay Recording & Post-Stage Replay Playback (`0x46c740..0x46e090`)**:
   - Wire `FUN_0046c7d0` keyframe ring buffer and `FUN_0046d8f0` RLE control recording into live `GameplaySession` so players can watch full stage replays and save `.SSR` files.
6. **2-Player Split-Screen (`DAT_00550c08 == 2`) & DirectPlay Network Session Wiring**:
   - Connect `crates/cmr-core/src/car_vs_car_collision.rs` (`FUN_00442740`) and `crates/cmr-net` into multi-car `GameplaySession` execution.
