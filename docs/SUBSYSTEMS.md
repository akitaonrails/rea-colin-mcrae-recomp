# Static subsystem map

This is a recovery backlog, not a claim that every imported API executes. Each
row begins with shipped-artifact observations and names the evidence needed to
define a clean compatibility interface.

| Boundary | Static anchors | Recovery question | Planned owner |
| --- | --- | --- | --- |
| launcher/video | `Game.exe`, COM interfaces, `CreateProcessA`, `HDPath`, `AVIPath` | What selects/plays intro video, and what exact conditions start `Rally.exe`? | `launcher` application |
| data roots | `HDPath`, `CDPath`, `AVIPath`, `WAVPath`, registry APIs | Which paths are required, and how are relative names normalized? | asset resolver |
| stored compression | zlib 1.1.3 strings; corpus gzip + `CMPR` envelopes | What bounds/errors/ownership does the loader use? | format crate |
| window/event loop | User32 window and message APIs | Which event ordering, focus, and pause rules affect simulation? | platform interface |
| graphics | DirectDraw, GDI palette APIs, image/surface source-path strings | What surface formats, palette rules, scaling, and flip timing are observable? | video interface + renderer |
| input | DirectInput plus WinMM joystick APIs | How are keyboard/joystick states sampled, calibrated, and mapped? | input interface |
| clock | performance counter, WinMM periodic timers, `timeGetTime` | Which clocks drive rendering, physics, menus, and audio? | clock/scheduler interface |
| sound effects | 16 `QSWaveMix*` imports, WAV assets; shipped DLL exposes 62 APIs | What channel, volume, frequency, position, and completion semantics matter? | audio mixer interface |
| music/video | WinMM MCI plus AVI/WAV path keys | Is playback file-, track-, or device-based, and what lifecycle is required? | media interface |
| persistence | `save\\cmRally.cfg`, registry reads/writes | What is mutable state, its layout, defaults, and failure behavior? | storage module |
| localization | language-file paths and ordered text tables | What indexing, encoding, fallback, and formatting rules apply? | localization module |
| models/scenes | C3D and stage-bundle path strings/files | What layouts and cross-file references build a drivable scene? | formats + scene loader |
| multiplayer | two DirectPlay ordinal imports, recovered IDirectPlay3A session surface | Which protocol/session behavior is actually reachable and safe to preserve? | optional transport behind the recovered host/join/player/message lifecycle: `NetworkTransport` + `SessionLifecycle` (loopback) and a direct-connect TCP backend (H10) |

## First focused queries

1. Complete the `Game.exe` registry-path helper and launcher state transition.
2. Trace `Rally.exe` entry and `HDPath`/`CDPath` references to recover source-root
   selection and path normalization.
3. Trace zlib wrapper entry points and one representative `CMPR` consumer.
4. Resolve mode-enumeration ordering, failure cleanup, and the later DirectDraw
   flip cadence now that creation and the first image/surface chain are mapped.
5. Trace QMixer initialization and one sound-load/play path.

Each result should update [EVIDENCE.md](EVIDENCE.md) and create or refine one
owned interface. Broad bulk decompilation is deliberately out of scope.

## Implemented contracts

- `cmr-formats::stored_asset`: bounded raw/gzip-`CMPR` storage decoding.
- `cmr-assets::AssetIndex`: safe Windows-style logical lookup against reviewed
  metadata.
- `cmr-core::startup`: loading/legal palette and bitmap selection for the four
  language directories present in the observed corpus.
- `cmr-formats::{bitmap,riff_palette}`: bounded decoding of the recovered
  startup BMP/RIFF-PAL subset, plus the exact 256-entry legacy palette contract.
- `cmr-core::surface`: backend-neutral indexed surfaces with explicit identity
  or recovered nearest-color index mapping against the active palette.
- `cmr-core::startup_surface`: end-to-end verified import, selected asset load,
  bounded decode, and indexed-surface construction with contextual errors.
- `cmr-core::blit`: recovered destination clipping, fast/stretch dispatch, and
  color-key flag planning without a platform graphics dependency.
- `cmr-core::display`: backend-neutral indexed display initialization with
  typed windowed/fullscreen topology, shared front/back palette ownership, and
  the recovered visible-front-then-back startup copy transaction.
- `cmr-core::source_store`: shared checked ownership for persistent indexed
  sources, including append-only logical keys, transactional upload rollback,
  explicit release, unwind cleanup, and reverse-order teardown; the font and
  fixed-slot root-bitmap pools are its first persistent resource clients.
- `cmr-core::root_menu_bitmap_raster`: fixed-slot surface preparation,
  transactional persistent upload, pool-to-visual binding, and checked
  per-command bitmap submission without reordering non-bitmap frame work.
- `cmr-core::root_menu_frame`: canonical transcript generation and strict
  sequential routing through recovered bitmap/footer executors; root C3D is
  planned at its transcript position and consumed once by the always-run
  terminal scene, followed by latch cleanup, the depth-gated deferred palette
  commit, and the distinct presentation boundary.
- `cmr-core::root_menu_c3d_scene`: backend-neutral depth-clear policy,
  view/viewport setup, BeginScene, optional prepared-batch submission, world
  reset, opaque ordered scene tail, EndScene cleanup, and selector-cache
  disposition, with checked prepared-pipeline provenance.
- `cmr-core::root_menu_c3d_visibility`: persistent six-slot raw visibility
  latches set only by successful prepared-frame planning and cleared after
  every terminal scene attempt before later frame-completion work.
- `cmr-core::window`: pure compatibility reducer for recovered Win32 message
  handling, activation-driven pause/restore state, queue dispatch decisions,
  and fullscreen-flip versus windowed-vblank presentation planning.
- `cmr-core::presentation`: platform-neutral execution of the recovered native
  presentation plan, including fullscreen sample-before-Flip invocation and
  windowed client-origin/vblank sequencing with subsequent-call VGA fallback;
  native window creation remains outside this contract. A bounded static
  search found no separate steady-frame windowed copy owner; any portable copy
  is therefore an explicit host policy rather than attributed legacy behavior.
- `cmr-core::deferred_palette`: recovered runtime-base logical mapping and
  current/pending palette state, plus the depth-8-only 256-entry terminal
  commit into an adapter and both observed software caches, now joined to the
  root terminal-frame orchestrator.
- `cmr-video::SoftwareIndexedBackend`: deterministic source-owned front/back
  framebuffers for exact-size indexed copies, recovered index-zero keyed glyph
  copies, and the bounded branch-zero root C3D software oracle; unrecovered
  stretch, alternate C3D profiles, and other color-key ranges are explicit
  boundaries rather than guessed behavior.
- `cmr-video::software_root_menu_session`: callback-scoped production assembly
  of the verified imported root resources, initialized software display,
  recovered disk-palette bootstrap, persistent source pools, C3D runtime state,
  and repeated complete headless root frames without a self-referential owner.
- `cmr-video::SoftwarePresentedFrame::digest_summary`: versioned canonical
  checksums for a complete indexed frame. The stream binds `CMRFRM01`,
  little-endian extent/pixel count, all 256 RGB-plus-flags palette entries, and
  row-major indices; component digests keep palette and pixel differences
  diagnosable. This identifies deterministic reconstruction output, not asset
  provenance or native display pixels.
- `cmr-app`: first native application boundary. It selects an external verified
  import only from `--runtime-data` or `CMR_RUNTIME_DATA`, embeds reviewed
  profile metadata but no payloads, initializes the software compatibility
  display/session, renders one fixed official-EU English/Normal root frame, and
  emits path-free counters and checksums. It is headless and one-shot; a shared
  frame/presenter seam must precede the SDL interactive host so this orchestration
  is reused rather than copied.
- `cmr-test-support`: non-publish synthetic imported-root fixture shared by the
  video vertical test and application integration test. It generates its own
  BMP/font/palette/C3D bytes, owns temporary source/import directories, and has
  no dependency on `cmr-video` or `cmr-app`.
- `cmr-core::config`: opaque exact-size legacy configuration admission,
  recovered volatile-byte clearing and save gate, plus safe registry-value
  parsing and caller-owned exact-record writing without assigning meaning to
  unknown bytes.
- `cmr-core::localization`: shared five-language identity, distinct frontend
  and in-game path policy, bounded byte-oriented table splitting, and safe
  indexed lookup with the recovered diagnostic fallback.
- `cmr-core::localization_assets`: verified import loading for the recovered
  fixed-width frontend and in-game text tables with path-aware failures.
- `cmr-core::input`: platform-neutral keyboard and WinMM-compatible joystick
  masks, rising-edge state, raw menu-key repeat, frontend combination policy,
  and focus-gated sampling. OS polling and force feedback remain backend work.
- `cmr-core::input_device`: the in-race driving-input **primitive** transforms
  recovered from the WinMM `joyGetPosEx` + `GetAsyncKeyState` poll (no DirectInput
  on this path). Reconstructs the device-record types, the rising-edge mask, the
  8-way digital direction from axis thresholds (`0x2710`/`0xd8f0`), the centred
  analog dead-zone rescale, the command saturate-then-quantize (clamp to `0xffbe`
  **then** `& 0xfffffc00`, so analog tops out at `0xfc00`), and the analog axis ->
  demand split into the `+0x2fc` command region; with the caps/offset/default-mask
  constants. Throttle is `+0x304` (digital bit 2, analog cap `0x2`, negative pedal
  axis); brake is `+0x308` (digital bit 3, analog cap `0x1`, positive pedal axis) -
  an earlier pass had the two inverted. The WinMM/Win32 acquisition is SDL-backend
  glue; the dead-zone source remains. The drivetrain arrays are resolved separately:
  raw per-wheel speeds `+0x2a8[4]`, smoothed/current values `+0x2b8[4]`, and gains
  `+0x2c8[4]`. See evidence `rea.official-eu.host-input-mapping`.
- `cmr-core::input_map`: the full stateful in-race control map `FUN_00449580`
  composed from the `input_device` primitives (B10/G02 boundary). Merges the
  player's primary device with the keyboard, writes the steer/throttle/brake/
  handbrake/gear/reset command fields of the per-car control block, applies the
  invert-steer swap and the alternate throttle ramp, and latches reset/rescue
  once per press. Carries the default P1/P2 keyboard binding tables and the
  per-action mask decomposition. See evidence
  `rea.official-eu.in-race-control-map`.
- `cmr-core::force_feedback`: the DirectInput force-feedback effect transforms.
  The shipped game drives FFB through DirectInput and that is the sole purpose of
  the `DINPUT.dll` import (resolving the B10 unknown): in-race input is WinMM,
  DirectInput is only the FFB output path (device `FUN_004aba70` ->
  `DirectInputCreateA`/`CreateDevice`/`SetDataFormat`/`SetCooperativeLevel`/
  `Acquire`; six effects created once, updated per frame via `SetParameters`
  only on changed fields from the double-buffered per-car block `DAT_00550040`).
  Reconstructs the magnitude rescale (`*0x2710>>16`, game `0..0x10000` -> DI
  `0..10000`), the collision peak+decay-step accumulator (copy-then-decay,
  clamp 0), the digital-wheel direction snaps (9000/27000) and the analog polar
  conversion, the rumble sample-period floor (`1000`us), the self-centring `/2`
  clamp, the cornering pre-shift clamp (`0x27100000>>16`), the `DICONDITION`
  build, the `DIEP_*` change-diff flag sets, the effect slot/kind table, and the
  enable gate (device && session && per-player bit `0x08`). DirectInput ABI
  constants are reproduced as public SDK values; effect GUIDs are represented as
  kinds, not vendored. The device/effect lifecycle is backend glue; the decay
  step source and per-source raw sums are G02/G03 force-wiring unknowns. See
  evidence `rea.official-eu.force-feedback`.
- `cmr-core::intro_video`: the intro / cut-scene FMV selection and playback-loop
  logic. The retail EU build plays video through Windows MCI (`avivideo` device,
  VfW `.avi`) - no DirectShow/Smacker/Bink, and the logos are static `.bmp`s.
  Reconstructs the MCI command/flag constants and composites (`OPEN_ELEMENT`
  0x30200, `WINDOW` 0x50000, `MM_MCINOTIFY` 0x3B9), the difficulty selector
  (`(cfg[+0x6c0]&0x18)>>3` -> EASY/MEDIUM/HARD/null) and language selector
  (`cfg[+0x18]&7`, reusing `localization::Language`) with the `avi_*` path
  components, the intro-script game-mode gate (`< 2`), and the `playback_step`
  state machine (`PumpMode` passive/interactive; `PlaybackStep` Continue/
  TogglePause/Finished/Aborted; `MM_MCINOTIFY` -> finished, ENTER/ESC -> abort,
  SPACE -> pause). Recorded negative result: `Rally.exe` does no command-line
  parsing (no skip-intro flag in this binary). The MCI device/window I/O and the
  `.rsp` interpreter are backend glue (H07). See evidence
  `rea.official-eu.intro-video`.
- `cmr-core::timing`: validated pure plans for recovered logical rates, timer
  startup/drift, compatibility buckets, focus gating, and unsigned deadlines.
- `cmr-core::host_loop`: the recovered native host clock and main-loop pacing as
  portable logic over a `HostClock` trait. Reconstructs the loop gating
  (`HostLoopState`/`HostAction`: active/quit/one-shot flags + the
  `WM_ACTIVATEAPP` focus-pause hinge), the self-limiting uncapped catch-up
  (`run_catch_up_ticks`: `do { tick } while (step_boundary_crossed)`, reusing
  `sim_clock`), and the one-frame render-skip gate (`RenderGate`, the
  `DAT_004ee900` 2<->1 toggle). Lock-step 25 Hz with decoupled render. The
  busy-wait spin and the Win32 message pump remain SDL-backend glue (B10-B12).
  See evidence `rea.official-eu.host-main-loop`.
- `cmr-formats::font`: bounded fixed-layout indexed FON atlas/trailer decoding
  plus byte-text measurement and backend-neutral glyph layout plans.
- `cmr-core::race_hud_plan`: pure normal single-player race-HUD command plan.
  Preserves the recovered gauge/progress/timer draw order, authored source and
  destination rectangles, Q16/pixel text coordinates, value formatting, and
  640x480 scaling. Optional layers remain typed gaps rather than guessed pixels.
  `cmr-app::RaceHudAssets` resolves the plan's identities through the reviewed
  user-owned asset store, CMPR-decodes and bounds-parses four indexed bitmaps
  and two FON atlases, and retains them with the native stage resources.
- `cmr-video::software_race_hud`: ordered indexed HUD compositor for the shared
  software frame. It implements source-index-zero transparency, ordinary and
  tinted diffuse behavior, center-nearest stretch, deterministic full-palette
  RGB quantization, direct-index FON glyphs, clipping, and checked source
  rectangles. `cmr-app` applies the planned HUD to initial and live world frames;
  the live speed/timer adapter uses recovered arithmetic while gear/revs and CAT
  progress remain explicit harness boundaries.
- `cmr-formats::stage`: the stage/track bundle (E08). A stage is a set of sibling
  files sharing a base path and differing only by extension (the 14-entry table
  `.c3d`..`.fuz`), each loaded through the shared CMPR-capable loader - no
  stage-level header. Decodes the recovered section payloads: the header-less
  `.cat` node spline (`0x1C`-byte records, count = filesize/0x1C; position /
  direction / split flag / seq index), the recovered mirror/reverse transform,
  the `{e,i,n}.cjp` per-difficulty split-time table (13 dwords), and the
  trigger-volume record (`0x30`: two points / Q16 plane normal / payload /
  spatial-node index / raw type / event flag / raw state / unknown byte). Pure,
  bounds-checked, rejects misaligned/truncated input
  with length+stride; the raw trigger type is classified by
  `cmr-core::collision::TriggerType`. The bundle front half ties these together:
  `load_stage(source: &impl StageSectionSource, c3d_limits) -> StageBundle` parses
  the mandatory `.c3d` scene plus optional `.dat`, `.obj`, `.sht`, `.hzn`,
  `.ars`, `.tsc`, `.csp`, and `.cat` sections from an abstract
  per-extension byte source (base-path resolution + file IO + CMPR decode stay in
  the caller), so a resolved stage base yields a scene ready for
  `cmr-core::c3d_mesh::assemble_scene_meshes` (`impl.stage-bundle-loader`). The
  remaining `.moj`, `.spl`, `.anm`, `.rsf`, and `.fuz` consumers plus DAT
  runtime dispatch, TSC director binding, and runtime fix-up/relocation remain. See
  evidence `rea.official-eu.stage-bundle`.
- `cmr-core::stage_triggers`: separates immutable DAT records from initialized
  runtime state; classifies recovered event families and evaluates the exact
  Q16 plane/XZ-segment crossing contract over current and prior eight-point car
  bounds. The compiled basis producer translates all eight probes into world
  space. The exact type-to-spatial-gate mapping and universal outer player-bit
  test are explicit, including type-1 ordinal-byte behavior. A table evaluator
  shares one prior snapshot across every record and copies current probes once
  afterward. Live mode predicates, event side effects, car-bound attachment,
  and first-frame prior initialization remain outside this pure contract.
- `cmr-core::car_resources`: the car-resource composition (E09). A car is a set
  of sibling `.C3D` resources under `OBJECTS\CARS\` (body `<code><DAY|NIGHT>.C3D`,
  reflection `<code><tag>ref.c3d`, shared `ref_map.bmp`, low-detail `med`/
  `daymed`), loaded through the same CMPR loader as stages. Reconstructs model
  selection (low-nibble index of `DAT_00d3ca80+0xb6+player*0x9c`, 14 models), the
  path builders, the scene node-id contract (`CarSceneNode` + `find_node_by_id`/
  `find_wheels` masked pre-order search = `FUN_00495050`; wheels are nodes 1-4,
  radius from node 6), the `wheel_radius` Q16.16 formula, and the damage
  addressing (`car_type_param_base` 0xcd0 / `damage_region_base` 0x334 /
  `nearest_unshed_panel`, reusing the G04 thresholds). Liveries = palette +
  DAY/NIGHT variant; damage = vertex deformation + panel-shed flags (no extra
  files). The car-code/make-name and variant panel tables are game data, not
  vendored. `release_car_data_slots` implements the 16-slot non-null free-and-clear
  teardown; nine tests cover the composition and lifecycle. E09 is complete. See
  evidence `rea.official-eu.car-resources`.
- `cmr-core::font_assets`: verified loading and stable style identity for the
  16 recovered frontend font atlases in original initialization order.
- `cmr-core::font_raster`: backend-neutral indexed atlas conversion and
  index-zero keyed execution of precomputed glyph plans with diagnostics, plus
  transactional lifetime ownership for the persistent 16-font upload pool and
  an identity-neutral prepare/execute seam that rejects fatal text work before
  backend effects. This is the direct indexed branch; the initialized
  alternate `0x90` source preparation, queued primitive, and optional driver
  `Load` path is now statically separated but not yet implemented.
- `cmr-core::root_menu_footer`: phase-gated localized footer layout and indexed
  execution into the caller-selected active frame target.
- `cmr-core::frontend`: checked first/root-screen navigation, transitions,
  idle/exit effects, and opaque target IDs.
- `cmr-core::championship_setup`: pure three-row car/gear setup state with
  explicit eligibility fallback, animation gates, profile commits on both
  forward/back transitions, and a backend-neutral preview transcript.
- `cmr-core::championship_assets`: verified-store admission of the 14 canonical
  car C3Ds and palettes plus 11 deduplicated manufacturer-logo bitmaps, with
  typed source-relative failures and no bundled payloads.
- `cmr-core::championship_bitmap_raster`: explicit active-palette-mapped
  preparation for the current 8-bit indexed output and transactional
  shared-store upload for the 11 unique aligned logo surfaces, preserving
  manufacturer/car aliases. This mapping is distinct from the recovered
  16-bit per-source embedded-palette conversion.
- `cmr-core::championship_setup_visual`: validated backend-neutral title/row/
  preview frame planning with exact recovered geometry, order, fonts, and
  paired visible-callback palette cache behavior.
- `cmr-core::championship_setup_frame`: whole-frame preparation and ordered
  execution for recovered setup text and 2D bitmaps, with source preflight,
  success-only cache commit, and typed deferred unrecovered commands. Its
  single ordered kernel now prepares either rich indexed payloads or a
  lightweight extent-only alternate stream. The public effect-free view
  preserves full text commands, exact bitmap source identities and blit
  decisions, and every deferred boundary without exposing indexed handles or
  pending cache state; exact-command parity tests bind both adapters.
- `cmr-core::championship_car_frame_plan`: exact-source, actor-scoped planning
  for one selected mode-8 car. It validates the complete topology/runtime join
  before advancing preview state, composes the recovered Q16 hierarchy only
  when visible, and owns active-level Type-2 batches in depth-first attachment order.
  Backend submission, palette transition, and pixels remain separate.
- `cmr-core::c3d_type2_fog`: pure owned-copy pre-draw mutation for the optional
  Type-2 word-5 path. It explicitly carries the retained process-global gate,
  thresholds, distance/center inputs, and x87 conversion contract; disabled state
  is caller-selected and is not inferred from frontend entry.
- `cmr-core::c3d_actor_rgb`: pure word-4 RGB packing that preserves alpha. The
  championship frame plan records the recovered recursive list-head owner for
  every draw, and preparation applies RGB after word-5 mutation while skipping
  the exact all-`-0x10000` origin sentinel path.
- `cmr-video::c3d_software_raster` and `SoftwareIndexedBackend`: explicit
  unified-indexed COPY/no-blend car profiles for all four word-5/actor-RGB gate
  combinations. It consumes owned prepared Type-2 words, copies sampled indices
  directly, maps reviewed-car missing
  selectors to index zero, rejects every unsupported runtime-state tuple, and
  preserves the root renderer's separate palette-quantizing policy. A
  backend-neutral pre-scene hook rejects absent, mismatched, or unsupported car state before
  depth clear, and the full terminal test covers draw, group-4 commit, and
  captured presentation.
- `cmr-video::c3d_software_raster` also retains the exact alternate renderer
  selection priority (`0x400`→16, `0x200`→24, `0x100`→32) and exposes a
  deliberately unintegrated canonical-RGB Type-2 MODULATE oracle. The
  `selected_texture_conversion_oracle` first materializes a nominal Palette8,
  RGB555, or RGB565 plane under an explicit no-dither policy; the compositor's
  type boundary requires that conversion before its named bilinear policy. A
  terminal source-owned compositor can pack depth-passing results through an
  independently selected logical framebuffer mask while preserving untouched
  words exactly. Driver `Load` behavior, native
  framebuffer layout, dithering, alpha, and backend presentation remain
  explicit boundaries.
- `cmr-core::root_menu_c3d_render_state` decodes the game's `0x00008000`
  capability-derived alpha gate, including its inverse pre-scene depth-clear
  decision and fixed source-alpha/inverse-source-alpha factors. It also records
  whether the requested branch filter is submitted or suppressed by the cache,
  preserving the distinction between a cached request and unknown effective
  device state. Alpha testing and alpha-enabled pixel composition remain
  unresolved and unintegrated.
- `cmr-video::legacy_c3d_pixel_format`: exact eight-dword legacy descriptor
  retention, evidence-bounded palette/RGB555/RGB565 candidacy, and separate
  safe mask validation. It retains separate palette and opaque-RGB callback
  slots and applies the explicit recovered palette gate without depending on
  renderer depth. Caller-supplied-mask pack/unpack is explicitly source-owned
  policy; driver conversion, byte layout, pitch, and dithering are not inferred.
- `cmr-video::ordinary_bitmap_rgb16`: the recovered ordinary `0x81` 8-bit-BMP
  source conversion, retaining complete aligned stored rows and each bitmap's
  embedded palette while packing exact discriminator-selected RGB555/RGB565
  logical words and the packed palette-entry-zero key. Its companion fast-blit
  module preflights matching-layout opaque/keyed logical copies, including
  aligned padding, before any write. A separate Stretch module preserves the
  recovered Selection rectangles, literal-zero key request, and layout checks,
  then applies an explicitly source-owned destination-cell-center nearest
  policy because DirectDraw sampling remains outside the executable. It is
  nominally separate from fonts, selected C3D textures, and framebuffer
  formats; native storage, driver-exact Stretch pixels, and the 4-/24-bit
  branches remain open.
- `cmr-video::software_championship_selection_rgb16`: nominal one-source owner
  that converts the exact common Selection bitmap once, retains its stable
  identity and media-relative path on failure, and prevents an ordinary
  info/logo/frame source from satisfying the Selection slot accidentally.
- `cmr-video::software_championship_ordinary_rgb16`: transactional fixed-shape
  ownership of converted R_INFO, 11 unique logo, and 15 Chequered sources,
  keyed by stable media-relative identities and retaining only contextual
  relative paths on failure. Selection Stretch and alternate fonts remain
  deliberately outside this pool.
- `cmr-video::software_queued_font_argb4444`: nominal persistent ownership of
  all 16 parsed fonts plus the exact aligned ARGB4444 source words produced by
  the bounded `0x90` decoder branch before optional driver `Load`. Exact RGB
  black alone receives zero alpha; other RGB receives alpha `0xf`, palette
  reserved bytes are ignored, and the installed source key remains literal
  numeric zero. This remains source preparation only: driver conversion,
  queue routing, blending, and final pixels stay outside the module.
- `cmr-core::font_raster::PreparedFontText` and
  `cmr-video::software_queued_font_glyphs`: source-neutral fatal validation
  plus lane-neutral semantic producer inputs for the alternate font path.
  Commands retain the raw four-byte tuple and signed Q16 fade; producer inputs
  retain the exact caller-side post-fade bytes, font style, source/destination
  rectangles, terminal `8` or `0x48`, command order, and glyph order without
  copying atlas ownership.
  `cmr-video::software_queued_font_queue` routes those inputs to a
  caller-selected typed lane, admits exactly 2,047 records independently per
  lane, tracks later silent drops, and applies the recovered byte-3 gate:
  preserve only when effective renderer mode is not 7 and the alpha-texture
  capability bit is set, otherwise force `0xff`. Safe out-of-atlas rejection
  remains a reconstruction policy. Native 52-byte packing, optional transform
  data, flush execution, filtering/blending, and pixels remain open.
- `cmr-video::software_queued_font_flush`: backend-neutral semantic flush plan
  for the current zero-rotation/no-viewport championship subset. It snapshots
  accepted records in recovered lane order 3, 2, 1 and per-lane FIFO order,
  attaches exact depths `0.999999`, `0.5`, and `0.0`, preserves the terminal
  split, resolves source owners, prepares exact bounded quad geometry, and can
  run a backend-specific validation pass before dispatch to an infallible
  typed sink. `software_queued_font_quad` supplies TL/TR/BL/BR corners, RHW,
  diffuse/specular carriers, and explicit renderer-mode UVs normalized against
  the aligned wrapper extent. The alternate terminal dispatches the prepared
  lanes after identity-world reset and before the common scene tail, including
  empty/Next scenes, and reports semantic counts. Native packing/count
  clearing, optional transforms/UV swaps, device states, blending, fallback
  pixels, and textured pixels remain outside this boundary.
- `cmr-video::software_championship_setup_frame`: two-pass execution of the
  lightweight prepared setup transcript. Its standalone preparation derives
  exact aligned extents from the owned RGB16 pool and optional nominal
  converted Selection source, with no indexed backend, uploads, fonts, or text
  table. Execution rejects stale preview-cache snapshots, preflights every
  effectful Fast bitmap and supported Selection Stretch before writes, and
  composes them in exact command order. Missing Selection at preparation stays
  typed deferred; losing the source after a Stretch was prepared is a typed
  preflight failure. Cache publication records completion of this bounded
  slice, not unresolved text/info/next-preview pixels.
- `cmr-video::software_championship_alternate_terminal`: whole-frame owner for
  the prepared RGB16 setup stream and optional selected-car scene on one split
  target. It preflights ordinary bitmaps plus the common scene and every car
  batch before writes, coordinates transform/latch/palette commands inline,
  runs a car or empty Next scene after setup, consumes car/root latches, and
  reports the non-indexed palette skip while retaining the pending request.
- `cmr-video::software_championship_alternate_session`: callback-scoped owner
  built directly from a verified `AssetStore`. It loads championship/frame/C3D
  assets, the selected frontend localization table, all 16 fonts, and the exact
  common Selection bitmap. It retains the font and 27-source RGB16 pools,
  converted split bindings, logical target/depth, executor cache, and all
  terminal state across repeated frames, and returns the owned headless backend
  after the callback. Each frame prepares every non-deferred setup Text command
  before terminal effects, routes it through caller-explicit lane/byte-3
  choices, and publishes the complete initially-empty bounded route for that
  non-deferred setup-text slice only in a successful outcome. RGB555/RGB565
  source-target and raster-state
  mismatches fail before callback entry. It creates no indexed backend, uploaded
  source, or native presentation surface. Selection is converted once and
  executed by the named source-owned Stretch policy; native queue flushing,
  glyph pixels, info text, and next-preview pixels remain deferred.
- `cmr-video::packed_rgb_target`: a nominal runtime-framebuffer format and
  logical masked-word target kept type-distinct from selected texture formats.
  It retains the complete raw surface descriptor and converts canonical RGB
  or a completed indexed plane plus explicit palette under validated masks,
  without exposing or inferring native byte order, surface pitch, dithering,
  palette provenance, or presentation. Completed-indexed-frame expansion is
  an integration oracle, not the recovered per-source bitmap conversion.
- `cmr-video::c3d_software_split_backend`: a standalone headless alternate
  backend that transactionally retains converted frontend bindings, owns an
  owner-scoped logical target plus depth buffer, preflights the complete frame
  including retained selector-cache coherence, and preserves color across
  depth-only clears. The championship alternate terminal now composes ordinary
  Fast bitmaps and the selected-car or empty scene in recovered order on this
  same target; full session ownership, native storage, and presentation remain
  separate boundaries.
- `cmr-video::software_championship_session`: callback-scoped standalone
  assembly retaining all 14 prepared car topology/runtime pairs, one shared
  binding executor, explicit raster-selection, optional source-ordered per-car
  actor-RGB tables, and 2D mapping policies, compact car/FCHE palettes,
  preview/transform/route state, and persistent uploaded sources. It validates
  every actor table against loaded topology, selects car resources only from
  the validated transcript, and releases every upload before returning the
  backend. The compact palette
  indices and supported software-oracle raster tuple are source-owned policy,
  not imported process-global state.
- `cmr-core::frontend_c3d_resources`: one frontend-session prepared arena with
  the exact 55-resource mode-8 car prefix followed by six fixed optional root
  slots, preserving cross-domain IDs, aliases, rollback, and diagnostics.
- `cmr-core::championship_frame_assets`: verified bounded loading of R_INFO,
  the 15 ordered Chequered preview frames, and FCHE, with explicit R_INFO
  mapping and transactional shared-source upload.
- `cmr-core::source_store::UploadedIndexedBitmap`: shared backend-neutral
  uploaded-bitmap identity and aligned extent used by root resources and ready
  for later frontend-specific pools.
- `cmr-core::indexed_bitmap_raster`: identity-neutral preflight and execution
  for persistent indexed bitmaps, including exact source-bound checks after
  legacy destination clipping.
- `cmr-core::root_menu_assets`: typed, verified-profile paths for localized
  root chrome, center icons, and normal/`g_` menu objects without asset bytes.
- `cmr-formats::c3d`: bounded, byte-preserving admission of the observed
  `CM3D` header and all three proven counted layouts, including raw group-0
  `u16` values, typed borrowed group-1 32/24-byte entries, and group-2
  label/blob pairs. The typed entries preserve raw bytes/offsets while exposing
  the field roles now **proven** by the draw-submission consumer `FUN_004a2b70`
  (E02): a 32-byte entry is one vertex (`position_q16` comps 0-2, `uv_q16` comps
  3-4, `normal_q16` comps 5-7, `q16_to_f32`) and a 24-byte entry is one triangle
  (`entry32_ordinals` = its three vertex indices, `group2_ordinal` =
  texture/material with the `C3D_ENTRY24_MATERIAL_NONE = -1` untextured sentinel
  and `is_textured()`, `corner_tuples`/`corner_colors` = per-vertex RGBA with
  byte 3 alpha, `field_22` = a per-primitive attribute word, `field_00`
  preserved-but-unconsumed), plus checked borrowed group-0→group-1, entry24
  `+2/+4/+6`→same-record entry32, and group-1-entry24→group-2 ordinal resolution.
  The draw-time UV wrap/bias is a render-time convention and is not baked into
  the parser. A group-0 record is also the scene-graph **transform/node** record:
  beyond `field_04_i16` and `affine_matrix_q16`, it now exposes the node
  descriptor fields `FUN_004a2b70` copies into the runtime node - `node_class`
  (`+0x48` -> node `+0x2c`), `node_id` (`& 0xfff`, the id `FUN_00495050` searches
  by), `is_translucent_class` (`== 0x6f`), `flag_a`/`flag_b` (`+0x4d`/`+0x4e`),
  `is_group_marker` (`field_04 == -1`), and `parent_index` (`field_04` when
  `>= 0`) - so the table projects to the `SceneNodeDescriptor` stream
  `cmr-core::c3d_scene_graph::SceneGraph::assemble` threads into the traversal
  arena. See evidence `rea.official-eu.c3d-entry-semantics`,
  `impl.c3d-entry-semantics`, `rea.official-eu.c3d-scene-node`, and
  `impl.c3d-scene-node`. Group-2 resources may be classified on demand as opaque
  `BM` bytes, checked borrowed `ANIM` frame references/thresholds, or unknown
  bytes without forcing universal bitmap decoding. A checked selected-record
  iterator resolves animation frame ordinals to generic group-2 records while
  preserving source offsets and later results after invalid ordinals. Runtime
  conversion and batching remain separate.
- `cmr-core::c3d_render`: the backend-neutral Direct3D-IM submission contract
  (E06). The shipped game renders through Direct3D Immediate Mode on
  `IDirect3DDevice2` (DirectX 5) with a hardware 16-bit Z-buffer - no software
  span-filler and no painter's triangle sort; ordering is a per-object
  **back-to-front** distance sort (farthest first, by 2D top-down XZ distance -
  `object_sort_key`/`object_drawn_before`, `FUN_004a2070`) plus scene-graph
  order, opaque geometry then a deferred transparent flush (the opaque/transparent
  split is deferral + an alpha-blend bracket, **not** a per-pass Z-write toggle -
  see `c3d_scene_graph`). The module encodes the public DX5
  render-state codes/values, the four-way `BlendMode`/`BlendConfig`
  (opaque/color-keyed/alpha/additive) from `FUN_004a2540`, the
  `ALPHABLENDENABLE`/`STIPPLEDALPHA` capability fallback, the `D3DLVERTEX`
  diffuse `d3dcolor_argb` bake with the untextured/transparent constants (the
  runtime prim, stride `0x1c`, holds each per-vertex color as ascending
  `[R,G,B,A]` bytes at `+0x08`/`+0x0c`/`+0x10`, repacked to `0xAARRGGBB` by
  `diffuse_from_rgba_bytes`; `FUN_004b1e10`), the batch-builder key
  (`prim_joins_batch` + the runtime-prim field offsets: texture handle `+0x04`,
  ordinals `+0x14`/`+0x16`/`+0x18`; `FUN_004b2340` groups consecutive prims
  sharing a handle, the topology `c3d_batch::plan_consecutive_batches`
  reconstructs), the
  `Pass` Z-write model, and the squared-XZ-distance `object_sort_key`, so the
  software oracle (E07) and a modern backend share one contract. The E02
  `field_22`/`prim+0x00` word is confirmed render-irrelevant (the batch builder
  keys on the texture handle). See evidence `rea.official-eu.c3d-rasterizer` and
  `impl.c3d-render`.
- `cmr-core::c3d_camera`: the backend-neutral view/projection stage (E05) that
  feeds the E06 submission. Projection is a left-handed `D3DMATRIX`
  (`perspective_lh`) built from a hardcoded FOV/aspect baseline (`2.0, 2.4, 1.0,
  250.0`), with a parallel sparse Q16.16 coefficient copy (`projection_coeffs_q16`)
  and a mirror view that negates the projection width; aspect enters only through
  the `Viewport` clip rectangle, never the FOV. Clipping is D3D hardware clipping
  with no manual near-clip. Culling is a 2D top-down XZ frustum
  (`point_in_triangle_xz` over three edge cross-products) with near/far extents
  and a near-list split, plus a terrain-cell radius cull; back-face culling
  defaults to CCW so front faces are clockwise, switching to `CULL_NONE` for a
  two-sided record. LOD selects one of several stride-`0x18` draw-group records
  under a global detail cap, with a `-1.0` sentinel triple marking a billboard.
  The camera VIEW matrix is the affine inverse of the camera-world matrix (not a
  look-at and not a BAM-angle composition): `invert_affine_q16` reproduces the
  shipped `FUN_0048f460` byte-exact (cofactor adjugate, integer `2^32/det`
  reciprocal, identity fallback for a singular basis) and `apply_affine_q16`
  reproduces the column-major affine transform. The routine is pure integer
  arithmetic, so it reconstructs exactly with no driver-float dependency - in
  contrast to the `cmr-video` pixel rasterizer, whose sequential-`f32` evaluation
  is stated reconstruction policy. The per-frame setup inverts the camera
  object's world matrix, converts it to a float `D3DMATRIX` (`matrix_q16_to_f32`,
  each word times `2^-16`), and submits it with `SetTransform(VIEW=2)`;
  `TRANSFORMSTATE_WORLD`/`VIEW`/`PROJECTION` = 1/2/3. WORLD * VIEW * PROJECTION is
  concatenated by Direct3D per vertex (no CPU matrix-matrix multiply in the
  opaque path - the reflection-UV path in `c3d_reflection` is the one exception),
  so the software oracle must perform that concatenation itself. The gameplay-scene
  submission builders `projection_matrix_words`/`world_matrix_words`/
  `view_matrix_words`/`viewport_words` emit the f32 `C3dMatrixWords`/
  `C3dViewportWords` the `cmr-video` software transform path consumes (row-major,
  row-vector, matching `multiply_row_vector`); the D3DVIEWPORT2 clip volume is
  scene-independent, so `viewport_words` reuses the shared `root_menu_c3d_viewport`
  builder (gameplay == menu viewport). `cmr-video`'s
  `SoftwareC3dTransformState::for_gameplay_scene` composes these builders into
  the raster's transform state, so a gameplay batch renders through the existing
  `preflight_root_c3d_batch` pipeline with gameplay matrices in place of the
  fixed menu matrices; an end-to-end test drives a synthetic C3D mesh through the
  real resource/frame pipeline and the gameplay matrices, asserting visibility,
  determinism, and distance-correct projection. A further oracle composites two
  objects at different node-Z depths into one shared framebuffer +
  `SoftwareC3dDepthBuffer` through the real `compose_root_c3d_indexed_copy_no_blend`
  and proves the hardware-Z-buffer contract: the final image and depth are
  identical regardless of submission order, the combined depth is the elementwise
  minimum of each object's solo depth, and the nearer object occludes the farther
  (`impl.c3d-occlusion-oracle`) - closing the deterministic compatibility
  renderer/oracle loop (transform, clip, 2D-XZ cull, Z-buffered raster,
  multi-object occlusion); only true differential verification against the
  shipped binary (a scoped runtime experiment) remains. The recovered C3D
  geometry chain now reaches this rasterizer directly: `preflight_mesh_lod_prim`
  maps an `assemble_lod_draw_group`/`assemble_node_mesh` prim's 32-byte vertices
  to the private `RasterInputVertex` (`from_mesh_entry`: model-space Q16.16
  dequant, COPY diffuse/specular `0`, node world left to the transform state)
  and feeds the shared `preflight_indexed_triangle_list` + `compose_*` path, so a
  parsed stage/car `.c3d` mesh renders through the same pipeline
  (`impl.c3d-mesh-raster-bridge`). See evidence
  `rea.official-eu.c3d-camera`,
  `impl.c3d-camera`, `rea.official-eu.c3d-view-inverse`, `impl.c3d-view-inverse`,
  `impl.c3d-scene-matrices`, `impl.c3d-scene-oracle`, and
  `impl.c3d-occlusion-oracle`.
- `cmr-core::c3d_scene_graph`: the backend-neutral gameplay scene-graph traversal
  and per-frame draw orchestration (E07), from the dispatcher `FUN_004a07d0`, the
  root emitter `FUN_004a2700`, and the recursive walker `FUN_004a28a0` (the walker
  cross-checked against its disassembly). `SceneGraph::emit_entity(root, pass)`
  reproduces the pass-gated pre-order walk: a node is visited only when its
  render-pass mask bit (`node+0x184`) is set; a visited, un-culled (`node+0x180`)
  node with a mesh (`node+0x0c`) emits a `DrawOp` (its `SetTransform(WORLD)` from
  `node+0x114`), then its children recurse; an entity root draws its own mesh then
  walks its child chain unconditionally; a translucent sibling (`node+0x2c` ==
  `0x6f`) is re-entered under a deferred blend bracket before the walk continues
  past it (transcribed exactly, including the mid-chain tail re-process the
  shipped terminal-translucent data avoids). The dispatcher is a single
  render-pass mask per frame (**no pass-index loop**) bracketed by
  `BeginScene`/`EndScene` (device vtable `0x28`/`0x2c`) with scene-wide
  `FOGENABLE`; it distance-sorts each object category when
  `should_distance_sort`, and collects near entities into a bounded deferred list
  (`DEFERRED_NEAR_LIST_CAP` = `0x19` = 25, the same `c3d_camera::NEAR_LIST_CAP`)
  flushed last under the alpha bracket. The capability gates are
  `alpha_blend_enabled(cap, flag)` (`cap != 7 && flag`), `node_prims_draw` (opaque
  always draws; translucent only when blending is enabled or the override is set),
  and `deferred_translucent_state` (additive vs enable). This resolves the
  render-capability globals (`DAT_00a23994` is the level `0..=7`) and corrects the
  E06 Z-write inference. The two per-object render-state brackets the dispatcher
  applies are also captured: `draw_texture_perspective` returns
  `TexturePerspective::Affine` for an inline (non-deferred) draw - restored to
  `GlobalDefault` afterward, with near-deferred objects drawn at the default - and
  `object_colorkey_disabled(cap, is_focus_entity)` force-disables `COLORKEYENABLE`
  for the distinguished focus entity (`param_3`) at render-cap level
  `RENDER_CAP_SORTED` only (the focus entity's gameplay role is not yet pinned);
  the `RS_TEXTUREPERSPECTIVE` = 4 constant lives in `c3d_render`. The traversal
  arena can now be **built from a real on-disk source**: `SceneGraph::assemble`
  threads an ordered `SceneNodeDescriptor` stream (parent index + node fields)
  into the arena by the shipped node-attach rule `FUN_00492410` - append each
  child as its parent's last sibling (first-child `+0x04`, else walk the
  next-sibling chain `+0x00` to the tail) - rejecting a forward/out-of-range
  parent with `SceneAssemblyError::ForwardParent`. The C3D Group0 transform
  table projects directly to that stream (see `cmr-formats::c3d` below), and
  `SceneGraph::from_c3d_group0` is the end-to-end bridge: it maps a parsed
  `C3dFile`'s Group0 records (1:1, as `FUN_004a2b70` does for well-formed files)
  into the arena - `has_mesh` from the mesh-index count, load-time `culled` the
  negation of `has_mesh` (node `+0x180` = 3 for a transform-only node), and the
  `ALL_RENDER_PASSES` default since the file carries no `+0x184` mask (the
  runtime assigns it). This closes the wiring from a real on-disk C3D source to
  the recovered traversal. See evidence `rea.official-eu.c3d-scene-graph`,
  `impl.c3d-scene-graph`, `rea.official-eu.c3d-scene-node`, and
  `impl.c3d-scene-node`.
- `cmr-core::c3d_terrain_cull`: the backend-neutral terrain-cell visibility grid
  cull (E05), from `FUN_00490040`/`FUN_00490af0`/`FUN_00490800`. The world is a
  rectangular grid of cells; each frame the cull rebuilds the visible-cell list
  the scene dispatcher iterates plus a 4096-bit cell mask. `CellGrid::cell_of_point`
  maps a world XZ point to its cell (`width * |z-anchor|/cell_size + |x-anchor|/cell_size`,
  clamped), `scan_box` is the clamped cell box of half-extent `radius_in_cells`
  around the camera's cell, `is_in_3x3_neighborhood` is the always-visible block,
  and a cell is otherwise kept when any of its four corners lies inside the
  view-cone triangle (`view_triangle`: apex = camera, two far corners from the
  forward direction) via `cell_corner_visible`, which reuses the same 2D-XZ
  `c3d_camera::point_in_triangle_xz` predicate the frustum cull uses (the shipped
  `FUN_00490800` is that predicate with the opposite cross-sign on a CW-wound
  cone, plus an AABB pre-reject). `CellVisibilityMask` is the 4096-bit per-cell
  bitset. The grid dimensions, cell size, world anchors, and far radius are
  per-track runtime data taken as parameters (never vendored), and the forward
  normalization's rsqrt LUT is not reproduced. See evidence
  `rea.official-eu.c3d-terrain-cull` and `impl.c3d-terrain-cull`.
- `cmr-core::c3d_reflection`: the backend-neutral spherical environment-map
  (reflection) UV generation (E07), from the translucent-node vertex-prep routine
  `FUN_0049f6b0` the scene-graph walker invokes for reflective meshes.
  `reflection_uv_matrix(node_world, camera_world)` builds the node-local-to-view
  transform as `matmul_q16(node_world, invert_affine_q16(camera_world))`;
  `reflection_vertex_uv` rotates a normalized node-local normal into view space
  (`transform_direction_q16`, the 3x3 rotation only) and `spherical_env_uv` maps
  the view-space normal to a UV pair `((n * 2^-16) + 1) * 0.5` (Z unused), written
  to the vertex `tu`/`tv`. The two matrix primitives are recovered into
  `c3d_camera`: `matmul_q16` (`FUN_0048ecb0`) is the **one CPU matrix-matrix
  concatenation** in the render path (the opaque path defers WORLD*VIEW*PROJECTION
  to Direct3D), and `transform_direction_q16` (`FUN_0048dfe0`) is `apply_affine_q16`
  without the translation term. The normal-normalization rsqrt LUT (`DAT_0082b380`)
  is not reproduced: the UV helpers take an already-normalized normal, as the
  object-sort and terrain-cull recoveries take pre-scaled inputs. See evidence
  `rea.official-eu.c3d-reflection` and `impl.c3d-reflection`.
- `cmr-core::c3d_mesh`: the runtime mesh layout and the per-node draw-call
  expansion (E06/E07), from the draw walker `FUN_004a28a0`. A drawable node
  selects its LOD (`mesh+0x80`), reads the draw-group header (`mesh+0x20 +
  lod*0x18`, vertex type at `+0x10`) and the prim-list head (`mesh+0x34 +
  lod*0x18`), and walks the singly linked prim records (ordinal/vcount/vbuf/
  icount/ibuf, next at `+0x18`). `draw_call(vertex_type, prim)` yields the
  `DrawIndexedPrimitive{ TRIANGLELIST, vertex_type, vbuf, vcount, ibuf, icount,
  DONOTUPDATEEXTENTS }`, `PrimBatch::is_untextured` tests the `-1` ordinal, and
  `prim_takes_fog` gates the per-prim fog bake (global flag on and vertex type
  not `1`). The buffers are opaque handles from the batch builder (`c3d_batch`);
  the per-prim translucent gate is `c3d_scene_graph::node_prims_draw`. It also
  recovers the loader node-loop half - how that draw-time layout is populated by
  `FUN_004a2b70`/`FUN_004ae1d0`/`FUN_004b1e10`/`FUN_004b2340`: the `0xa4`-byte
  runtime record's vertex count (`+0x0c`), triangle count (`+0x10`, doubled for
  the two-sided menu mode `3` when the renderer lacks hardware two-sided handling
  - `runtime_face_count`/`mode_duplicates_back_faces`), the runtime triangle array
  (`+0x18`, stride `0x1c` - `RuntimeTriangle`), the dequantized staging vertices
  (`+0x00`, stride `0x20`), and the stage/car LOD-0/LOD-1 vertex-stream backups
  (`+0x04`/`+0x08`, modes `5..=8` - `preserves_lod_vertex_streams`/
  `lod_stream_backup_offset`), plus the vertex-type diffuse split
  (`vertex_type_has_per_vertex_diffuse`: lit `2` vs pre-lit `1`). Decisively, the
  prim builder `FUN_004b2340` allocates vbuf/ibuf from a system-memory arena (not
  a Direct3D buffer) and its topology is the existing
  `c3d_batch::plan_consecutive_batches`, reused rather than re-derived; the only
  Direct3D resource the loader creates is the texture. The parse -> batch ->
  draw-group wiring is closed by `assemble_lod_draw_group(record, vertex_type,
  selectors) -> MeshLodDrawGroup` (prim build delegated to
  `c3d_batch::plan_consecutive_batches`; selector = each face's resolved group-2
  material ordinal), with `MeshLodDrawGroup::{takes_fog,has_per_vertex_diffuse}`
  and `prim_draw_counts(prim) -> (vertex_count, 3*triangles)`. The node ->
  multi-LOD mapping is `assemble_node_mesh(mesh_indices, group1_records,
  vertex_type) -> AssembledMesh`: each node mesh-index entry (`node+0x50`,
  count `node+0x4c`) is one LOD selecting `group1_records[index]`, a `0xffff` or
  zero-geometry index leaving an empty (`None`) LOD slot, the active LOD `0` at
  load; with `node_mesh_vertex_type(load_flags)` (`param_5 & 1`: `2` lit / `1`
  pre-lit, shared by all LODs). The whole-graph capstone
  `assemble_scene_meshes(group0_records, group1_records, vertex_type)` resolves
  every node's `C3dGroup0Record::values()` through the group-1 LUT into one
  `Option<AssembledMesh>` per node (aligned with the `from_c3d_group0` node
  index; meshless nodes -> `None`), closing parse -> scene graph -> world
  transforms -> per-node multi-LOD mesh -> per-LOD draw groups ->
  `DrawIndexedPrimitive`. See evidence `rea.official-eu.c3d-rasterizer`,
  `rea.official-eu.c3d-mesh-instantiate`, `impl.c3d-mesh-draw` and
  `impl.c3d-mesh-instantiate`.
- `cmr-core::c3d_transform`: the per-frame scene-graph world-transform
  propagation (E07/E08), from `FUN_00492190`/`FUN_00492520`. The loader stores
  only each node's local matrix; this pre-pass composes the world matrix down the
  tree as `world = local * parent_world` (`c3d_camera::matmul_q16`, row-vector),
  with two special cases - a world-root parent (`parent+0x2c == 1000`) makes the
  child take its local matrix as world, and a node flagged at `+0x188` inherits
  the parent world verbatim - and a focus-entity (`DAT_00b77f4c`) rotation-to-
  identity reset (translation kept). `propagate_world_transforms(nodes,
  root_world)` returns each node's world matrix; roots compose against the
  external container world (identity by default). The draw walker submits a copy
  of this matrix (node `+0x114`) as the Direct3D WORLD transform; the runtime
  dirty-flag/`+0x180`-kind scheduling gates are left to the caller. See evidence
  `rea.official-eu.c3d-world-transform` and `impl.c3d-world-transform`.
- `cmr-core::c3d_batch`: checked, backend-free topology planning over one
  group-1 record and caller-prepared selectors. It preserves maximal
  consecutive selector runs and source entry order, deduplicates local entry32
  ordinals by first use within each run, and emits compact three-index streams
  while retaining borrowed typed entries. Its owned neutral draw command then
  carries a checked ordinal, explicit Type-1/Type-2 branch, selector, compact
  runtime vertices/side vectors/indices, and numeric primitive/flag constants;
  root commands remain lossless metadata adapters.
- `cmr-core::root_menu_c3d_resources`: policy-driven, backend-neutral
  preparation of embedded C3D bitmaps into an owned 2,048-entry arena. It
  accepts car or root identities plus explicit append/intern/material/layout
  policies, SHA-binds each prepared object to its exact admitted C3D bytes,
  preserves selectors and raw DIB metadata, and rolls back failed objects.
  Root fixed-slot preparation remains a parity-tested adapter.
- `cmr-core::c3d_topology`: crate-private single-object topology join that
  verifies source SHA-256 and selector arena before borrowing ordered group1
  records, resolving checked group0 references, and preserving consecutive
  selector runs and compact first-use ordinals. Public car/root APIs choose the
  source/prepared pair rather than exposing an arbitrary join.
- `cmr-core::root_menu_c3d_topology`: combined fixed-slot root preparation and
  metadata/error adapter over the domain-neutral topology seam.
- `cmr-core::root_menu_c3d_runtime`: domain-neutral resource-scaled CPU
  materialization through recovered wrapping Q16/f32 conversion, explicit load
  policy, pre-mutation side capture, type-1/type-2 mutation, first-allocation
  surface-layout UV scaling, first-usable-resource visitation, and compact
  batches. Root mode 2 remains a parity-tested adapter; the championship path
  closes over its SHA-checked mode-8 policy and fixed `0x96969696` word.
  Backend submission, palette integration, and rasterization remain separate.
- `cmr-core::root_menu_c3d_transform`: deterministic root-only fixed-point
  transform words with source-owned quarter-wave trigonometry, integer-derived
  square-root normalization, exact raw-angle branch behavior, persistent Q16
  placement, f32 world/view/projection words, and checked viewport construction.
  Scene traversal, binding, drawing, and rasterization remain separate.
- `cmr-core::root_menu_c3d_frame_plan`: transactional exact-root validation and
  non-cloneable owned one-frame submission snapshots, retaining source batch
  order, prepared-arena provenance, exact transforms, and numeric draw words.
  Backend resource binding and terminal scene execution remain separate.
- `cmr-core::root_menu_c3d_execute`: generic arena-scoped opaque RAII binding
  catalogs, whole-command resource preflight, and a commit-after-bind,
  explicitly invalidatable selector cache, plus the root source-ordered
  world/bind/draw adapter. Missing selectors remain explicit until a backend
  implements the recovered material/texture-zero operation. One frontend
  catalog can own car and root resources; scene/depth/latch/presentation
  orchestration remains separate.
- `cmr-core::frontend_c3d_scene`: shared checked frontend C3D transaction for
  optional depth clear, identity view, viewport, BeginScene, one owned
  submission, identity-world reset, opaque tail, EndScene, and best-effort
  post-Begin cleanup. The root API remains a compatibility adapter.
- `cmr-core::championship_car_execute` and `championship_car_scene`: exact
  actor-world/resource-or-default/draw submission for a preflighted car frame,
  including explicit missing-selector material/texture-zero binding and a
  persistent selector cache. Prepared frames are tied to the exact binding
  catalog and their planned extent; uncertain scene completion invalidates the
  cache.
- `cmr-core::frontend_c3d_route`: session-scoped ownership of the shared C3D
  backend binding state. Root and championship terminal owners invalidate the
  executor they enter only on first use or a route switch, while same-route
  frames retain the recovered selector cache.
- `cmr-core::championship_terminal_frame`: complete preflighted championship
  setup boundary. It executes deferred transform/latch/palette callbacks inline
  with 2D work, always runs the common scene (including an empty Next preview),
  clears all 14 car then six root latches, commits the deferred palette, and
  presents. Typed failures preserve the exact completed prefix and suppress
  later terminal work.
- `cmr-core::root_menu_c3d_assets`: exact six-slot optional loading of normal
  or `g_` root-menu objects with ordered asset/header diagnostics and stable
  holes, plus explicit selection-slot mapping and six independent wrapping
  frame-angle states bound to the exact parsed asset.
- `cmr-core::root_menu_visual`: exact phase-gated root chrome geometry, Q16
  backdrop crop and center placement, and an ordered asset/object transcript.
- `cmr-core::root_menu_animation`: pure activation, wrap, tween-schedule, and
  transition-suppression state for the root screen.
- `cmr-core::root_menu_bitmap_assets`: verified, bounded loading of required
  and optional indexed root bitmaps with retained optional diagnostics.
- `cmr-core::root_menu_bitmap_raster`: fixed-slot identity preparation using
  the active palette and unchanged indices, with typed optional/required
  conversion policy and no backend side effects.
- `cmr-core::root_menu_frame`: ordered bitmap/footer dispatch plus parsed C3D
  slot binding and angle advancement; the software backend now executes the
  bounded root C3D branch and captures the composed indexed frame through a
  headless terminal hook. Native Flip and synchronization remain host
  boundaries; a portable windowed copy is reconstruction policy because the
  shipped steady-frame path exposes no observed copy owner.

The canonical binary maps registry language values 0–4 to English, French,
German, Spanish, and Italian startup paths. The supplied corpus has no Italian
directory, so this profile intentionally exposes only its four present
languages rather than fabricating or silently aliasing Italian assets.

## Binary roles and variants

- `Rally.exe` is the gameplay executable: PE32 i386, fixed image base
  `0x400000`, relocations stripped, Visual C++ 6-era linker.
- `Game.exe` is a VC5-era DirectShow/COM intro-video launcher.
- `qmixer.dll` is QSound 3D Audio Wave Mixer 3.00 middleware over waveOut and
  optional DirectSound. The official-EU `Rally.exe` audio module lives at
  `0x4957d0`–`0x495a80`: `0x4957d0` initializes a 22050 Hz, 30-channel QSound
  session via `QSWaveMixInitEx`/`QSWaveMixActivate`; `0x4958e0` builds a PCM
  `WAVEFORMATEX` from (name, rate, bits, stereo) and registers it with
  `QSWaveMixOpenWave` into a fixed wave-slot table; `0x495a30` plays a slot on a
  channel through `QSWaveMixPlayEx`. Per-channel control wrappers follow at
  `0x495b70` (volume), `0x495bd0` (frequency), `0x495c30` (3D position),
  `0x495b10`/`0x495b30` (stop), and `0x495d10` (channel-busy query); panning is
  3D-position based over the 30 channels. See evidence
  `rea.official-eu.audio-wave-path` and `rea.official-eu.audio-mixer-controls`.
- The reconstruction-owned mixer interface for this surface is
  `cmr-core::audio_mixer`: a backend-agnostic `AudioMixer` trait plus a pure
  deterministic `MixerSessionState` that reproduces the recovered init guard,
  the 22050 Hz/30-channel cap, the incrementing wave-slot table, the PCM
  `WAVEFORMATEX` build, and the per-channel volume/frequency/3D-position/stop
  records as emitted `MixerCommand`s, all checked by a shared
  `check_audio_mixer_contract`. B03 covers all 16 reachable imports; the other
  46 exports are explicitly unreferenced. The software/SDL
  audio backend both remain to be implemented against this interface. See
  evidence `impl.audio-mixer-contract`.
- The weather/crowd ambience loader `0x41cd30`/`FUN_0041cd30` registers six
  waves via the load wrapper at 11025 Hz / 16-bit: `sounds\crowd\crowd.wav` and
  `sounds\weather\abduct.wav` unconditionally, then `clap1`/`clap2`/`rumble1`/
  `rumble2` only at the storm weather level (`FUN_0046b230()==3`). This recovered
  inventory is encoded as metadata in `cmr-core::audio_weather_assets`. Thirteen
  further load-wrapper callers (engine, surface, UI, …) remain to enumerate. See
  evidence `rea.official-eu.audio-weather-wave-inventory` and
  `impl.audio-weather-assets`.
- The ambience trigger tick `0x41cfe0`/`FUN_0041cfe0` drives those waves on one
  shared channel: a proximity-summed crowd volume (clamped to 32000) plays the
  crowd slot looping (`QSWaveMixPlayEx` flag `0xffffffff`) while crowd objects are
  in range, and a storm selector plays a one-shot thunder variant (flag `0`) at
  `base+2+selector`. The volume setter `0x47eb70`/`FUN_0047eb70` applies a 0..100
  master-volume percentage (`DAT_004f653c`) over the recovered `0x495b70`
  wrapper. The recovered play flags, master-volume scale, volume clamp, and
  thunder selector are encoded in `cmr-core::audio_mixer` and
  `audio_weather_assets`. See evidence `rea.official-eu.audio-ambience-trigger`.
  A third source, the abduct trigger `0x41d270`/`FUN_0041d270`, plays slot
  `base+1` one-shot at fixed volume `0x7fff` and, with the storm flag, suppresses
  the looped crowd voice while it holds the shared channel (crowd plays only when
  `DAT_0050d868==0 && DAT_0050d860==0`). Only the crowd slot loops. See evidence
  `rea.official-eu.audio-abduct-trigger`.
- Both ambience ticks are reconstructed as pure command-emitting functions in
  `cmr-core::audio_ambience::AmbienceController`, checked against the recovered
  branch behavior (evidence `impl.audio-ambience-controller`). The per-frame
  scheduler that calls them, and the thirteen other wave loaders and their play
  trigger sites, remain to recover.
- The full recovered sound namespace (string table) is catalogued in
  `cmr-core::audio_assets`: explicit waves (crowd, six weather incl.
  `wet_out`/`rain_in`), `frontend`/`replay` music, menu sounds, and the
  `car`/`accident`/`prangs`/`newsurfs`/`speech`/`rallschl` directory groups. See
  evidence `rea.official-eu.audio-sound-taxonomy`.
- The menu UI sound loader `0x468f50`/`FUN_00468f50` registers four 22050 Hz /
  16-bit mono waves - `FORWARD`/`BACKWARD`/`LEFTRIGH`/`UPDOWN.WAV` under
  `sounds\menu\` - on a dedicated full-volume menu channel (`DAT_0057c608`, base
  slot `DAT_0057d88c`), inferred frontend navigation sounds. The separate
  `BACK`/`SELECT`/… strings are not this loader's waves. See evidence
  `rea.official-eu.audio-menu-loader`.
- The complete bare wave-filename inventory is recovered (string table): the
  car engine model (twelve cars × six `{RT,FT,RM,FM,RS,FS}` loops + shared
  gear/ignition/pop/chatter/whine, loaded by `0x4872b0`), the co-driver pacenote
  call corpus, the surface/collision material set, extra menu waves, tutorial
  numbers, and per-language cheat/secret sounds. The structured car model is in
  `cmr-core::audio_car_assets`. See evidence `rea.official-eu.audio-wave-inventory`.
- The car loader body `0x4872b0`/`FUN_004872b0` is recovered: it selects the
  player car from the low nibble of a config byte (`DAT_00d3ca80+0xb6`) via a
  switch remap (row 0 = SUBARU), allocates slots `+0..+7` (and `+8..+0xf` when
  `DAT_00550c08==2`), and loads 14 waves at 11025 Hz/16-bit/mono in order
  `FS,RS,FM,RM,WHINE,FT,RT,CHATTER,POPLOOP1-4,IGNITION,GEAR`. The format, load
  order, and selector remap are in `cmr-core::audio_car_assets`. See evidence
  `rea.official-eu.audio-car-loader`.
- The engine tick `0x487e00`/`FUN_00487e00` (+ whine sub-tick `0x488ab0`) drives
  the looping engine layers: each layer on its own channel
  (`base + slot + player*8`), updated per frame in volume and pitch. Three rev
  bands are stereo pairs (Slow 0/1, Mid 2/3, Top 5/6; WHINE 4, chatter 7), front
  slot pans one way and rear the other. The band index is
  `((RPM-lo)*count)/(hi-lo)` with out-of-window muting (pitch `0x2b11`) and
  overlapping windows giving the crossfade; the Top band is throttle-gated
  (`DAT_00829548`). Crossfade primitives are in `cmr-core::audio_engine`. F/R is
  a stereo-pan split, not on/off power. See evidence
  `rea.official-eu.audio-engine-crossfade`.
- The menu play helpers (`0x467ef0`, `0x467dd0`, siblings) each set full volume,
  stop the menu channel, and play `base+offset` one-shot: offset 0 `FORWARD`
  (confirm, carved) and offset 3 `UPDOWN` (vertical nav, carved); offsets 1
  `BACKWARD` and 2 `LEFTRIGH` by slot order. Reconstructed in
  `cmr-core::audio_menu::play_menu_sound`. See evidence
  `rea.official-eu.audio-menu-play`.
- The surface and collision sounds are two further banks, each with its own
  slot/channel base. The collision/prang loader `0x469ac0` reserves 5 channels at
  full volume `0x7d00` and loads 48 named `prangs`/`accident` waves; impacts play
  one-shot via `0x469e80` (speed picks a sub-wave and scales volume, a named
  material wave layering on `channel+1` for hard objects `0 < objType < 0x11`).
  The surface/skid loader `0x4892b0` (`newsurfs`) reserves 4 looping channels and
  loads per-object-type variant waves named `<type>_o/i/sko/ski<n>.wav` (surface
  outer/inner, skid outer/inner); roll play `0x46a030` ramps volume
  `base/3 + (count-5)*base/0x2d` to full at `count>=0x24`. Both banks are
  11025 Hz/16-bit mono. The roll ramp, variant naming, and bank constants are in
  `cmr-core::audio_surface`; the impact sub-wave/volume bit-expression is left in
  the evidence ledger pending a confident transcription. See evidence
  `rea.official-eu.audio-surface-collision`.
- The co-driver pacenotes (speech) are a 40-wave bank loaded once by
  `0x444160` ("CoDriver_Create") at 11025 Hz/16-bit/mono, slot offset == load
  index (slot 0 `CAUTION` ... 0x27 `INTO`, with the number calls `1..5` at
  3..7). Each path is `<root>` + `<language dir>` + `codriver\` + filename; the
  language is `cfg[0x18] & 7` over `english/french/german/spanish/italian/helium`
  (`helium` is the forced pitched-up developer voice). Calls play one component
  at a time, one-shot, on a single shared speech channel gated by
  `IsChannelDone`; a packed "pacenote word" carries several fields the sequencer
  `0x444450` extracts (direction 1=RIGHT/2=LEFT, a number, and higher fields of
  unknown meaning), linking consecutive queued notes with `INTO`. An 8-entry
  event ring feeds a depth-2 per-track FIFO with eviction-cancel. Reconstructed
  in `cmr-core::audio_pacenote` (inventory, format/channel constants, language
  gate, `codriver\` path, and the observed field-extraction arithmetic); the
  higher-field semantics and FIFO accumulation are left in the ledger. See
  evidence `rea.official-eu.audio-pacenote`.
- Music (`frontend.wav`, `replay.wav`) is loaded whole - not streamed - through
  a second loader variant `0x4959f0` (managed handle table, fixed format args,
  vs. the base `0x4958e0` rate/bits/stereo loader) and played looping; the
  frontend start `0x443b20` plays once and the keep-alive `0x4c9d70` re-starts
  the loop when the channel reports done and fades it in. All music is gated by
  the sound-disabled flag `DAT_00d3ca88`. The start-of-stage countdown
  (`5-4-3-2-1-GO`) is six `countdwn\` waves preloaded by `0x46a740` ("PreSnd")
  into consecutive slots at 11025 Hz/16-bit/mono under `<speech base>` +
  `<language dir>` + `countdwn\`, played one-shot as slot `base+index` by
  `0x46af10` on a shared SFX channel (cadence driver `0x439b10`); rally-school
  mode adds the `intro2\` set. The channel-setup helper `0x4958a0` (open +
  enable) is shared by 15 call sites across every audio bank, and `0x47eba0` is
  a second master-volume bus beside the known `0x47eb70`. Reconstructed in
  `cmr-core::audio_music_cues` (music tracks/flags, countdown set/format/path);
  the countdown slot ordering/cadence and replay/SETUP control flow are left in
  the ledger. See evidence `rea.official-eu.audio-music-countdown`.
- Music selection and lifecycle (H05), in the same `cmr-core::audio_music_cues`.
  **Decisive negative result: the retail EU build plays no CD / Redbook / MCI
  music** - the WINMM MCI import drives only the `avivideo` FMV device, never a
  `cdaudio` device; music is exclusively the QSound PCM path (`frontend.wav`,
  `replay.wav`). Selection is context-driven, not a table: `DAT_00d3ca88 == 0`
  (frontend context) gates frontend music and the countdown
  (`frontend_music_active`); the frontend context loops `frontend.wav`, replay
  loops `replay.wav`, in-stage has no music. Start sets the volume from config
  `DAT_00d3ca80+7` (`music_volume`: `(byte&0x7f)*400000/100`) and plays with loop
  count `0xffffffff`; the keep-alive `FUN_004c9d70` (`music_keepalive`) fades in
  toward `(cfg2[0xf]&0x7f)*0x14` while playing and reloads+restarts on drain.
  This resolves the earlier "volume scaling units unknown" note (the scaling is
  recovered; absolute QMixer units stay mixer-backend policy). No CD subsystem is
  implemented. See evidence `rea.official-eu.music-lifecycle`.
- `cmr-core::music_player::MusicController` (H07, music backend): drives the H05
  lifecycle as `MixerCommand`s over the `audio_mixer` contract, so the software
  mixer / SDL audio device play looping frontend/replay music with no new device
  code. `on_context` starts frontend music on the gate (SetVolume + Play loop),
  `keepalive` restarts on drain / advances the fade ramp / holds, `stop` tears
  down. The fade curve and replay trigger are H05 unknowns. See evidence
  `impl.music-backend`.
- `cmr-core::video_player` (H07, video backend): the `VideoBackend` compatibility
  interface (open/set_bounds/play/toggle_pause/close, mirroring the MCI
  `avivideo` calls), `apply_step` (PlaybackStep -> backend action), and
  `run_movie` (open/play then fold `intro_video::playback_step` over an input
  sequence -> Completed/Skipped). The real MCI/decoder and the live input poll
  are runtime glue. See evidence `impl.video-backend`.
- The first concrete audio backend `cmr-core::audio_render::SoftwareMixer`
  implements the recovered `AudioMixer` contract as a portable, I/O-free PCM
  mixer: it renders per-channel voices (one-shot/loop, linear volume gain,
  `frequency/output_rate` playback step, nearest-neighbour resample, mono/stereo)
  into an interleaved stereo buffer at 22050 Hz, drawing decoded waves from a
  `WaveSource`, and passes the shared `check_audio_mixer_contract`. Volume/
  frequency/position realisation is a documented design decision (the QMixer
  scales are unknown); 3D position is recorded but not yet spatialised. A later
  platform/SDL device backend wraps it by decoding the user's WAVs into the
  `WaveSource` and pumping `render` into the device. See evidence
  `impl.audio-render`.
- `cmr-formats::wav::parse_wav_pcm` decodes the game's PCM `WAVE` files (RIFF,
  PCM mono/stereo at 8/16-bit) to interleaved `i16`, bounds-checked and
  rejecting malformed/truncated input with field-specific errors while retaining
  unknown chunks and trailing bytes; `cmr-core::audio_render::PcmWave::from_wav`
  bridges a decoded wave into the software mixer. This is the decode half of the
  WAV -> `WaveSource` -> `render` playback chain; the remaining half is the
  file/directory wave source and the SDL audio-device wrapper. See evidence
  `impl.audio-wav`.
- `cmr-core::audio_render::FileWaveSource` is the directory-backed `WaveSource`:
  it maps a game wave name to a safe relative path under the user's source root
  (backslash-normalised, traversal/absolute/drive/NUL refused, regular-file
  only), reads the WAV in place and decodes it with the PCM parser. It completes
  the WAV -> `WaveSource` chain; only the SDL audio-device wrapper (opening a
  22050 Hz stereo device and pumping `render`) remains for live playback. See
  evidence `impl.audio-file-wave-source`.
- `cmr-platform-sdl3::audio::AudioHost` is the SDL3 audio-device wrapper: it
  opens a 22050 Hz/stereo/s16 playback stream and, in the SDL callback, locks a
  shared `SoftwareMixer<FileWaveSource>` and pumps `render` into the device
  buffer. The game drives the recovered subsystems against the shared mixer
  handle (`AudioHost::mixer()`). This closes the audio chain end to end:
  recovered trigger logic -> `AudioMixer` commands -> software mix -> WAV decode
  -> SDL device. The device glue is runtime-only (not unit-tested); everything
  it relies on is contract/unit-tested in `cmr-core`/`cmr-formats`. See evidence
  `impl.audio-sdl-device`.
- `cmr-core::audio_mixer::apply_command`/`apply_commands` bridge the recovered
  command-emitting controllers to any `AudioMixer` backend by dispatching each
  `MixerCommand` to the matching method (`OpenWave` rejected - loads go through
  `load_wave`). An end-to-end test drives `audio_menu::play_menu_sound` through
  the bridge into the `SoftwareMixer`, confirming the recovered stop-then-play
  shared-channel behaviour produces the expected samples. This is the seam that
  lets the game's recovered audio logic play through the software/SDL backend.
  See evidence `impl.audio-command-bridge`.
- The driving simulation runs on a fixed 25 Hz / 40 ms step. WinMain
  `0x49db00` pumps Windows messages and calls the frame driver `0x4c1e00` ->
  HighLevelTimer tick driver `0x443e90` -> FSM tick `0x443db0`, a
  return-next-state machine on enum `DAT_005590a8` over a handler table at
  `0x4ee830`/`0x4ee840` (ids 2..0xc; in-race states `5..=10`). The step is
  `timeGetTime` integer-millisecond bucketing (`t/0x28`, 0x28 = 40 ms) with
  catch-up and a decoupled present pump - no float dt accumulator. The in-race
  update `0x43afd0` ("Mechanics") loops the active cars (`DAT_00550e90`).
  Reconstructed in `cmr-core::sim_clock` (step constants, bucket/boundary
  predicates, in-race gate). See evidence `rea.official-eu.sim-fixed-step`.
- The simulation's numbers are signed 32-bit Q16.16 fixed-point (`0x10000`=1.0;
  canonical `(i64)a*(i64)b>>16` multiply; dt coefficient `0xa3d`~0.04) with
  16-bit BAM angles into a 4096-entry trig table `DAT_0082d380`
  (`sin=table[a&0xfff]`, `cos=table[(a+0x400)&0xfff]`); floats appear only at
  `__ftol` input boundaries. `FUN_0048fba0` generates the trig, atan, and
  square-root tables at startup with x87 operations and truncate-toward-zero
  conversions. Reconstructed in `cmr-core::fixed_point` (`Fixed16` and BAM
  indexing) plus deterministic `cmr-core::legacy_math::{sin_q16,cos_q16,sqrt_q16}`;
  no game table bytes are copied. `Fixed16::sqrt` is deliberately the distinct
  mathematical integer-square-root helper. Primary car state base `DAT_00700300`
  (stride 0x768); control/telemetry base `DAT_005fffc8` (stride 0x464). See
  evidence `rea.official-eu.sim-fixed-step`,
  `rea.official-eu.sim-runtime-math-tables`, and
  `impl.legacy-math-tables`.
- The per-car vehicle dynamics run from the dispatcher `0x407690` (current-car
  globals `DAT_0050c604`/`DAT_0050c5b4`): input smoother `0x4088c0`, gearbox
  `0x41b5b0` (auto `0x41b810` / sequential `0x41bb30` / mode-2 `0x41bf00`), RPM +
  rev limiter `0x40d0c0`, torque `0x40ce40`, and the force/velocity integrators.
  Gear is a signed byte (`0..=6` forward, `7` reverse; zero ratio = neutral);
  auto-shift compares RPM against per-gear `ratio*redline` thresholds; the rev
  limiter clamps `[0,redline]` with a `min(margin,1.0)*0x51eb` cut; engine torque
  is `throttle*maxTorque - engBrake*rpm^2`, delivered as `torque*ratio*finalDrive`
  split across the wheels; the tachometer/engine-audio RPM is `norm*6500+2000`
  clamped `[2000,8500]`. Reconstructed in `cmr-core::drivetrain` (all Q16.16).
  Per-car-type params live at `~0x50e170` (stride 0xcd0), selected by the
  car-type byte `car+0x738`. Raw per-wheel speeds are `+0x2a8[4]`, smoothed/current
  speeds `+0x2b8[4]`, and smoothing gains `+0x2c8[4]`. A later force-pass branch
  now has a recovered signed raw candidate-selection rule: retain the current
  candidate at/below its threshold, otherwise use a caller-derived replacement
  verbatim. Both lookup-free branches and the lookup-dependent branch are now
  reconstructed: `derive_lookup_dependent_candidates` composes the neutral
  signed-ratio phase, deterministic trig and legacy square-root lookups, bases,
  and gains without assigning physical meaning. The later `+0x420` correction
  gate's pure signed scalar rules are also reconstructed. Direction/state
  accumulation and parts of the torque-to-velocity integration remain. The
  ratio arrays are now known to come from fourteen compiled initializer cases,
  not a runtime file loader. `cmr-core::compiled_car_initializer` records the
  exact eight-state/twenty-three-control destination schema and unsigned
  selector behavior without embedding the case values, then separately derives
  the later eight-slot reciprocal/copy families from caller-supplied post-callee
  words. `cmr-formats::pe_compiled_initializer` now proves the reviewed i386
  PE32 seed/dispatch/jump-table and exact store grammar before returning all
  fourteen direct case records. `cmr-assets extract-compiled-car-initializer`
  identifies the user-owned executable before parsing and atomically publishes
  the versioned 1,760-byte `CMRCCASE` file plus path-free state;
  `CompiledInitializerStore` revalidates and owns those records without
  retaining a filesystem path.
  `cmr-core::compiled_car_initializer::CompiledDirectCaseSet` now proves
  per-record word counts through fixed array types, explicitly checks selector
  count and exact offset-schema identity at that boundary, copies every signed
  word in selector order, and exposes the existing unsigned direct-case
  selection contract over a core-owned set. It deliberately does not apply the
  separate reciprocal helper to these pre-common-path words.
  `FUN_00404160` is now recovered as the next narrow stage: it transforms only
  `+0x1d4,+0x1d8,+0x1dc,+0x1e0,+0x1e4,+0x1e8`, leaving `+0x1d0/+0x1ec`
  unchanged. It forms all five wrapping adjacent differences from the original
  run, masks the caller-owned option byte to three bits, chooses the observed
  five Q16 scale/bias policies, and cumulatively rebuilds the run with signed
  arithmetic-shift products and wrapping adds.
  `transform_compiled_control_run` and
  `transform_compiled_control_family` encode that payload-free contract; six
  synthetic tests bring `cmr-core` to 1,112. The preceding common-path helper
  `FUN_00404100` is recovered too: it masks config-record byte `+0x95` to three
  bits and unconditionally writes `control+0x170` as Q16 `0x4000`, `0x8000`,
  or `0xc000` for modes `0`, `1`, or `2..7`. Because `+0x170` is not a direct
  case destination and every selector reaches this call, invalid selectors
  still receive the helper-derived word. Downstream `FUN_0040d810` consumes it
  and its `0x10000-split` complement as the center drive split in both coast
  and driven distribution. `compiled_center_split_q16` encodes only the total
  byte-to-word policy; two tests cover all low-bit policies and all 256 bytes,
  bringing `cmr-core` to 1,114. `FUN_00407270` is now bounded as a disjoint
  22-dword auxiliary patch: seven policy immediates at
  `+0xf8,+0x10c,+0x110,+0x120,+0x124,+0x128,+0x13c`; nine common immediates
  at `+0xd8,+0xdc,+0xe0,+0xe4,+0xe8,+0xec,+0xf0,+0xfc,+0x104`; and derived
  words at `+0xf4,+0x100,+0x108,+0x118,+0x11c,+0x12c`. A force flag selects
  default without config lookup; otherwise modes `1`, `2`, and `0,3..7` choose
  policy one, policy two, and default. Derived words use signed Q16 products,
  arithmetic `state+0x48 >> 3`, and three zero-guarded signed reciprocals.
  `AuxControlConstants` keeps all official function literals caller-owned;
  `derive_aux_control_patch` emits the exact ordered schema and returns typed
  atomic overflow. Six tests bring `cmr-core` to 1,120. The official constants
  now have a separate reviewed user-owned extraction/store boundary:
  `pe_aux_control_initializer` semantically validates the complete canonical
  function without a shipped-byte template and returns the neutral 38-word
  shape; `CMRAUXC` v1 stores those signed words in an exact 184-byte container.
  The exact-digest metadata-only official profile and explicit
  `extract-aux-control-constants` command hash the bounded executable before
  parsing, reject linked/special paths, self-check a fresh stage, and atomically
  publish fixed path-free data/state under ignored `runtime-data/`.
  `AuxControlConstantsStore` revalidates every identity, count, size and hash,
  rejects malformed/truncated/trailing/substituted files, and owns the constants
  without a path. Verification comprises 149 format tests, 76 assets-library
  tests, eight CLI tests, three audits, and an ignored `CMR_ORIGINAL_DIR` smoke
  limited to public shape/counts. `AuxControlConstantSet` is now the checked
  core ownership adapter: compile-time array equalities pin record counts,
  construction verifies exact format/core offset order, and all 38 signed words
  are copied into core-owned records that survive dropping the store and root.
  Its narrow `derive` method delegates to the existing auxiliary-patch helper
  and preserves typed reciprocal overflow. Three tests bring `cmr-core` to
  1,123. The shared signed option lookup is now recovered and bounded too.
  Resolved state constructors write only `0..=7` to `state+0x738`; the three
  helpers sign-extend it, multiply by stride `0x9c`, and read raw option bytes
  at record-relative `+0x93`, `+0x95`, and `+0x99`. Unindexed
  `config+0x26 & 2` alone forces the auxiliary default. The runtime config
  block is an initialized `0x6c4`-byte heterogeneous object: slots zero and one
  are directly authored/persisted in a two-stride window, but later indexed
  addresses overlap other copied regions, so neither two nor eight is claimed
  as a homogeneous table count. `resolve_compiled_config_options` rejects
  negative and `>=8` slots, bounds the caller-owned byte view through the
  highest required read, preserves raw bytes for the existing helpers to mask,
  and does not mutate input. Five tests bring `cmr-core` to 1,128. The recovered
  non-contiguous stages now compose as separated effects. Static dataflow shows
  that the omitted prefix always adjusts direct control `+0x148`, can Q16-scale
  direct control `+0x148/+0x15c` and all eight direct state words under config
  flags, and derives state `+0x48/+0x4c`, but leaves the raw eight-word
  family, center split, and auxiliary region untouched. After the three helpers,
  omitted `FUN_00486090` writes only state `+0x758`, disjoint from those effects.
  The also-omitted `0x406324..0x406459` interval then writes state `+0xec`;
  control dwords `+0x144,+0x14c,+0x150,+0x154,+0x160,+0x174,+0x178,+0x17c`,
  `+0x180,+0x184,+0x188,+0x18c,+0x1a8,+0x1b4,+0x408`; control byte `+0x440`;
  and control word `+0x3e4`. Its guarded `+0x15c`-to-`+0x160` reciprocal can
  fault before the reciprocal/copy tail. At G03-X,
  `compose_recovered_compiled_stage_effects` returned the raw direct outcome,
  center scalar, auxiliary patch, and four family/tail arrays separately; it
  required caller-supplied post-prefix `state_word_48`. Config, auxiliary, then
  family validation produced typed atomic failure only among those represented
  stages. It deliberately was not a complete state/control snapshot, and
  success did not prove the shipped full path reached or published the tail.
  Five more tests brought `cmr-core` to 1,133. G03-Y then reconstructs and
  integrates the exact common prefix `[0x4060d3,0x4062ba)`: an exact-write
  patch carries unconditional wrapped control `+0x148`, optional bit-`0x02`
  control scaling, optional bit-`0x04` scaling of all eight direct state words,
  and derived state `+0x48/+0x4c`. Conditional writes remain optional rather
  than fabricated stores; reciprocal overflow at divisors `-1`, `1`, or `2`
  is typed and atomic. The config resolver now returns raw unindexed `+0x25`,
  and stage composition derives the prefix for valid or invalid selectors,
  feeds its `+0x48` to the auxiliary patch, and validates represented stages
  config-before-prefix-before-aux-before-family. Eight new tests bring
  `cmr-core` to 1,141. G03-Z then reconstructs exact no-call interval
  `[0x406324,0x40645f)` as a separate mixed-width patch: fifteen control
  dwords, one state dword, one control byte, and one control word. It consumes
  effective post-prefix `+0x148/+0x15c`, copies `+0x148` to `+0x14c`, and
  derives the guarded signed reciprocal at `+0x160`. Composition preserves the
  optional scaled-versus-raw `+0x15c` dependency and extends represented-stage
  precedence with PostHelper before Family. Five new tests plus strengthened
  composition tests bring `cmr-core` to 1,146. G03-AA then reconstructs
  `FUN_00486090` and its initializer selection: the conditional unindexed
  config `+0x6c0` gate chooses `-1` or a caller-owned table value; non-sentinel
  dwords store verbatim to state `+0x758`, while sentinel resolves checked
  record `+0x91 & 7`. Four new tests plus strengthened resolver/composition
  tests bring `cmr-core` to 1,150. The second direct caller and its `+0x738`
  save/restore narrow the initializer claim to the only observed originating
  constructor assignment. Table values/extent, merged object application,
  meanings/units, config loading, application/session/gameplay wiring, runtime
  reachability, generic second-caller provenance, and other editions remain.
  G03-AB completes the later derived pair in exact no-call interval
  `[0x406655,0x4066dd)`: signed Q16 product of effective control `+0x15c` and
  reciprocal-family slot 6 (`+0x208`) becomes control `+0x168`; its arithmetic
  quarter drives the guarded signed reciprocal at `+0x164`. The shipped store
  order is `+0x164` first, then a reload/recompute and `+0x168`; the pure helper
  is pair-atomic only under stable-memory assumptions. Composition runs this
  after family success and reports exact `IDIV` overflow divisors `-1,1,2` as a
  typed final-stage error. Four new tests plus strengthened composition coverage
  bring `cmr-core` to 1,154. Semantics, complete aliases/consumers, concurrency,
  runtime reachability, merged object application, wiring, and other editions
  remain open. G03-AC maps the remaining instruction-aligned suffix through
  exact function end `0x407231` and reconstructs its first bounded call-free
  slice `[0x4066dd,0x406831)`. The slice arithmetic-halves effective state
  `+0x20/+0x24/+0x28` only in temporaries, wrapping-accumulates their Q16
  squares, and reuses the exact generated-table `sqrt_q16` for state `+0x44`.
  It writes raw integer seeds `[1,2,1]` to `+0x50/+0x54/+0x58`, doubled to
  `[2,4,2]` by config `+0x25 & 0x04`. Three new tests plus strengthened
  schema/composition coverage bring `cmr-core` to 1,157. The mapped later
  call/pointer/RNG/snapshot blocks remain separate open scopes. G03-AD then
  reconstructs `[0x406831,0x4068dc)`: effective state `+0x20/+0x24/+0x28`
  sequentially derives guarded signed reciprocals at `+0x38/+0x3c/+0x40`.
  Typed overflow identifies exact fault divisors `-1,1,2`, the failing slot,
  and earlier shipped stores while the pure aggregate stays atomic. Three new
  tests plus strengthened schema/composition coverage bring `cmr-core` to
  1,160. G03-AE reconstructs the next `[0x4068dc,0x4068ff)` clear loop as an
  ordered sixteen-dword zero effect. Its eight iterations interleave the
  `+0x31c..+0x338` and `+0x33c..+0x358` families; zero is carried in `EDI` from
  the preceding interval, and the shipped pointer reload/partial-publication
  behavior remains outside the pure stable-object contract. One focused test
  plus strengthened schema/composition coverage bring `cmr-core` to 1,161.
  G03-AF combines the next provisional slices because their register setup
  overlaps: `[0x4068ff,0x406aaf)` writes 27 arithmetic-half/sign state dwords,
  ten control dword zeros, four state u16 zeros, and four state dwords selected
  as zero or `-0x8000` by raw unindexed config `+0x26 & 0x02`. Three focused
  tests plus strengthened resolver/schema/composition coverage bring `cmr-core`
  to 1,164. The pure effects assume stable objects/config and do not model the
  outgoing `ESI`/`EBX` handoff or native partial publication.
  G03-AG maps `[0x406aaf,0x406b00)` as the strict five-stage call/clear/global
  spine. Its eight u16 clears and three global seeds have a tested caller-local
  schema, but remain outside the composed stage because all three calls mutate
  state and two have unresolved transitive callees. `cmr-core` has 1,165 tests.
  G03-AH reconstructs the first call plus its sole helper as reusable pure
  stable-snapshot contracts: a pointer-backed 3×3 matrix and three half words
  generate four primary sign vectors and four center reflections, then two
  vectors/three scalars adjust only the primaries. Six focused tests bring
  `cmr-core` to 1,171; native
  aliasing/partial publication and full spine composition remain open.
  G03-AI reconstructs the exact caller-side eight-lane postprocessor after
  explicit geometry-dispatch results: drive gating, exhaustive type classes,
  first-four subtraction, accumulator/latch behavior, and per-lane config
  override. Six focused tests bring `cmr-core` to 1,177. The global-table
  dispatcher remains an explicit open boundary rather than a fabricated pure
  lookup.
  G03-AJ reconstructs that dispatcher's outer handle state machine behind an
  injected geometry provider. Exhaustive signed-handle routing and all normal
  branches/failure prefixes are tested without claiming the provider's global
  tables or geometry arithmetic. Three focused tests bring `cmr-core` to 1,180.
  G03-AK1 reconstructs the build arithmetic over decoded vertices/type, with
  exact store order, normalization/cross/plane rules, fallback, and typed final
  division overflow after publication. Five focused tests bring `cmr-core` to
  1,185; raw table lookup and spatial candidate search remain open.
  G03-AK2 reconstructs `FUN_00447540`'s inner candidate chooser over a signed
  count and bounded handle list. It preserves lazy validation/decoding,
  wrapping signed-average distance, first-winner/strict-replacement ordering,
  stable ties, and final-only handle publication. Five tests bring `cmr-core`
  to 1,190; the outer spatial hierarchy and raw geometry lookup remain open.
  G03-AK3 reconstructs outer `FUN_004473f0` hierarchy resolution over decoded
  cells: wrapping X/Z mapping, signed bounds, row-major level zero, five
  two-bit refinements, exact `-1` sentinel/depth behavior, and lazy cell bounds.
  Four tests bring `cmr-core` to 1,194. Candidate-pool adaptation and raw table
  ownership/composition remain open; the deepest native pointer slot aliases
  the vertex-table pointer and is represented only as a decoded cell view.
  G03-AK4 composes that locator with a candidate-word pool and the AK2 chooser
  while retaining nonpositive-count bypass and lazy provider/list bounds.
  Three tests bring `cmr-core` to 1,197. Raw official table acquisition and a
  concrete validate/decode/build provider remain open.
  G03-AK5 reconstructs the decoded `FUN_00446d00` predicate: signed inclusive
  X/Z bounds followed by three winding-sensitive, independently projected Q16
  edge scores with wrapping arithmetic. Six tests bring `cmr-core` to 1,203;
  raw record/vertex decoding and provider integration remain open.
  G03-AK6 reconstructs `FUN_00446e80` as an exact type-free vertex decoder
  over safe stable tables. Three tests bring `cmr-core` to 1,206. G03-AK7 then
  composes the hierarchy/pool, decoder, validator, chooser, separate post-decode
  type read, and builder behind a concrete stable-table provider. Five tests
  bring `cmr-core` to 1,211. The composition repeats logical resolution at each
  phase but does not claim equivalence to native global-pointer rebinding,
  candidate mutation, aliases, or component-level fault prefixes. G03-AK8
  binds one provider snapshot per lane into the ascending `FUN_00419a40` loop,
  immediately applies that lane's caller stages, and performs the config read
  late through a fallible interface. Four tests bring `cmr-core` to 1,215 and
  pin concrete decoded-provider execution plus dispatch/config failure state
  prefixes. Raw official table acquisition, native alias/rebinding faults,
  `FUN_00408fe0`, and full call-spine composition remain open. G03-AK9 then
  proves those tables are the decoded stage `.hpc` sidecar, not C3D or
  executable-resident data. `cmr-formats::hpc` parses and validates the five
  cell levels, vertices, records, candidate pool, and dimensions while
  retaining every reserved word; an opt-in test validates the user-owned
  corpus in place. The core adapter safely supplies an empty unreachable sixth
  level instead of reinterpreting the native vertex-pointer alias. Opaque
  trailing payload bytes are retained. Eight synthetic format tests bring
  `cmr-formats` to 158; `cmr-core` remains at
  1,215 with its parsed-HPC eight-lane integration. Stage/application wiring
  and the following global transform remain open. G03-AK10 then adds
  `stage_compiled_geometry::StageCompiledGeometry`, an owned runtime resource
  loaded only from the reviewed `AssetStore` using an explicit
  profile-relative stage base. Stored bytes are re-hashed and CMPR-decoded by
  the existing store before parsing; no basename search, raw-media fallback, or
  new extractor is introduced. Its eight providers borrow the same immutable
  snapshot and feed the AK8 executor. Seven focused tests bring `cmr-core` to
  1,222. Live stage identity and real simulation-input wiring remain open.
  G03-AK11 reconstructs the following no-argument `FUN_00408fe0` transform:
  selector words choose one of three four-point lane quads; two wrapping-Q16
  face vectors are upward-oriented and normalized; the first is published at
  state `+0x324..+0x32c`, then the normalized sum overwrites it. The pure
  stable-snapshot effects retain both publications and the ignored EAX square
  sum. Five focused tests bring `cmr-core` to 1,227. Native reloaded-pointer
  aliases/fault prefixes and post-AK8 orchestration remain open.
  G03-AK12 composes the recovered barrier: a typed post-stage interface runs
  exactly once after eight-lane success, never after dispatch/config failure,
  and retains the complete lane success if the post stage fails. The concrete
  normal adapter consumes original source vectors and final post-config lane
  words. Five focused tests bring `cmr-core` to 1,232. Later caller work and
  native pointer/fault behavior remain open.
  G03-AK13 reconstructs the call-free post-normal basis rebuild
  `FUN_00404440 [0x406b00,0x406e30)`: wrapping-Q16 dot of the AK11 final
  vector with the prior `+0x258` vector, projection removal, generated-table
  normalization, and the cross in exact `u × n` order. Stores publish the
  verbatim AK11 row at `+0x264`, the rebuilt row at `+0x258`, and the cross
  row at `+0x270` in native order; old rows are write-only. The concrete
  `CompiledPostLaneBasis` adapter consumes the AK11 final publication only
  after eight-lane success. Ten focused tests bring `cmr-core` to 1,242. The
  separate `[0x406e30,0x406e9a)` pointer publication and native alias/fault
  behavior remain open.
  G03-AK14 reconstructs the publication: `publish_compiled_post_lane_basis`
  writes the rebuilt `+0x258` row, the verbatim AK11 `+0x264` row, and the
  cross `+0x270` row at destination strides `+0x00/+0x10/+0x20`, three dwords
  each, preserving the padding dwords. `CompiledPostLanePublishedBasis`
  composes AK11, AK13, and publication after eight-lane success. The
  following `FUN_00409c10(state)` call edge is documented. Three focused
  tests bring `cmr-core` to 1,245. Destination aliases, pointer rebinding,
  and progressive fault prefixes remain open.
  G03-AK15 opens the state-adjustment phase: the wrapping accumulation
  `state+0xcc += state+0x2e4 - state+0x100`, then publication of `+0xc8`, the
  updated `+0xcc`, and `+0xd0` at destination `+0x30/+0x34/+0x38`, preserving
  the following dword. The subsequent call order `FUN_00492770`,
  `FUN_00409c10`, `FUN_004126b0` is pinned; only `FUN_00409c10` is already
  reconstructed. Two focused tests bring `cmr-core` to 1,247. The remaining
  callees and native alias/fault behavior remain open.
  G03-AK16 reconstructs the reset block: six state dwords zeroed at
  `+0x3cc..+0x3e0`, then `+0xbc = 0`, `+0xc0 = 0x10000`, `+0xc4 = 0`, and the
  descending control loop clearing eight dwords at `0x3b0..0x3cc` in native
  order. `derive_compiled_adjustment_reset` pins the constant effects. Two
  focused tests bring `cmr-core` to 1,249. Config-dependent flags, copies,
  and final state writes remain open.
  G03-AK17 reconstructs the config-selected control flag at `+0x3f0` (only
  config bit `0x10` participates) and the constant writes: control
  `+0x3f4/+0x3f8/+0x3fc/+0x3ec` zeroed, control bytes `+0x43c/+0x43d/+0x43e`
  zeroed, control `+0x37c = -0x4ccc` and `+0x41c = 0`, state
  `+0x684/+0x610/+0x678/+0x6e4` and u16 `+0x60c/+0x614` zeroed, and three
  dwords copied from `+0x258` to `+0x5d4`. Two focused tests bring `cmr-core`
  to 1,251. The `+0x6f8` flag and `+0x6e8/+0x6ec` words remain open.
  G03-AK18 reconstructs the conditional `+0x6f8` flag (short-circuited second
  callee), `+0x6f0 = 1`, and the `+0x6e8/+0x6ec` words: zero when
  `DAT_00550c08 == 2` or `DAT_00d3ca88 != 0`, otherwise `+0x6e8 = 1` and
  `+0x6ec` is the signed remainder of the RNG draw by two times ten plus
  `0xd`, reachable values `3`, `13`, and `23`. Two focused tests bring
  `cmr-core` to 1,253. The copy loops and final state writes remain open.
  G03-AK19 closes the direct effects of `FUN_00404440`: 24 dwords copied
  from `+0xfc` to `+0x1bc`, 9 dwords from `+0x258` down to `+0x21c`, then
  `+0x240` receives `+0xc8/+0xcc/+0xd0` and `+0x24c` the AK11 final vector.
  State `+0x74d/+0x74e` and control `+0x428` are zeroed, control `+0x420 = 1`,
  `DAT_0050c5fc = ((~config_25 & 1) << 0xf) | 0x1999`, and `FUN_0046eda0()`
  runs only when `DAT_00d3ca88 != 0`. Three focused tests bring `cmr-core` to
  1,255. The remaining callees and full call-spine composition remain open.
  G03-AK20 reconstructs `FUN_0048d2d0`: false at detail `8`; otherwise
  weather `3`/`4` yields `DAT_00592efc != 1`, weather `5` yields true, and
  every other input yields false, with native short-circuit evaluation of the
  weather and intensity getters. One focused test brings `cmr-core` to 1,256.
  The `FUN_00492770` switch stores remain open.
  G03-AK21 reconstructs `FUN_00492770`: cases `0`/`3` store the source
  vector at `+0xc4/+0xc8/+0xcc`, cases `1`/`2` add stores at
  `+0x10/+0x14/+0x18`, other values store none, and every path zeroes
  `param_1+0x188` and calls `FUN_00492050`. One focused test brings
  `cmr-core` to 1,257. `FUN_00492050` remains unreconstructed.
  G03-AK22 reconstructs `FUN_004126b0`: starting from `+0x742 = 0` and
  `+0x743 = 1`, it scans lanes `7` down to `0` with the strict signed test
  `lane_word + 0x1999 < source_y` (wrapping addition). Passing lanes set
  their `+0x73a` flags; failing lanes clear them and `+0x743`, append their
  indices at `+0x744 + count`, and increment `+0x742`. Two focused tests
  bring `cmr-core` to 1,259. The `FUN_0046eda0` global copy gate remains open.
  G03-AK23 reconstructs `FUN_0046eda0`: only when `DAT_00550c08 == 1`, it
  copies `0x119` dwords from `DAT_005fffc8` to `DAT_0060042c` and `0x1da`
  dwords from `DAT_00700300` to `DAT_00700a68`, each destination beginning
  immediately after its source range. One focused test brings `cmr-core` to
  1,260. The copied global tables are runtime data; full call-spine
  composition follows.
  G03-AK24 reconstructs `FUN_00492050`: it follows the node link at `+0x8`
  and sets the `+0x170` flag to `1` on every node until the null link, doing
  nothing for a null initial pointer. `apply_compiled_92050` reproduces the
  effect over an injected flag slice. One focused test brings `cmr-core` to
  1,261.
  G03-AK25 composes the post-lane spine in native order:
  `execute_compiled_spine_post` runs the eight-lane executor (aborting on
  failure), AK11, AK13, the AK14 publication, the adjustment and its
  publication, `FUN_00492770`, the lane thresholds, the reset, the control
  flags, the conditional flags and RNG word, the copies, `DAT_0050c5fc`, and
  the final call gate, all through the pure callee contracts. Two focused
  tests bring `cmr-core` to 1,263. Progressive faults, pointer rebinding, and
  the early initializer stages remain open.
  G03-AK26 completes the composition: `compose_compiled_initializer` chains
  the early post-dispatch stages, the basis transform, the call-spine locals,
  and the eight-lane/post-lane spine into one typed result. An early failure
  returns before any lane runs; a lane failure aborts before the post-lane
  spine. Three focused tests bring `cmr-core` to 1,266. The exact native
  dataflow into the injected inputs remains a separate boundary; wiring to
  real imported stage and car/config inputs is next.
  G03-AK27 proves the composed initializer over an authenticated snapshot:
  a CMPR-wrapped reviewed `.hpc` loads through `StageCompiledGeometry` and
  drives `compose_compiled_initializer` to completion with the reviewed
  synthetic config. All eight lanes finish with the decoded record type and
  built output vector, and the AK11 normal, AK13 basis, and copies are
  independently re-derived from the final lane words. One focused test brings
  `cmr-core` to 1,267. The car config record remains a reviewed synthetic
  fixture; recovering its car-data source and live stage identity is next.
  G03 wheel-slip force and saturation: the per-wheel loop of `FUN_0040d810`
  forms `slip = dot * 4 - wheel_speed` (wrapping), computes
  `qmul(slip, DAT_0050c600) * 4`, clamps it symmetrically to the per-wheel
  limit at `ctl+0x378`, then downshifts by `5`, `2`, or `3` depending on the
  per-wheel flag and the two config dwords. `cmr-core::drivetrain` gains
  `wheel_slip_candidate`, `slip_sign_disagreement`, `clamp_wheel_slip_force`,
  and `wheel_slip_shift`. Four focused tests bring `cmr-core` to 1,271. The
  slip-force gate, per-wheel drive application, and wheel-speed integration
  remain open.
  G03 wheel drive application, slip gate, and speed integration: the drive
  application adds the axle split and wheel constant on negative speed,
  leaves force unchanged at zero, and subtracts on positive. The slip gate
  applies slip for front wheels always and rear wheels only without handbrake
  or with sign disagreement. Integration is `speed + qmul(force,
  DAT_0050c5c0)`, followed by the zeroing gate (sign flip when `gear == 0`,
  `ctl+0x40c != 0`, or `ctl+0x158` equals the per-wheel constant; otherwise a
  gear-ratio sign mismatch). `cmr-core::drivetrain` gains
  `apply_wheel_drive`, `apply_wheel_slip`, `integrate_wheel_speed`, and
  `gate_wheel_speed`. Four focused tests bring `cmr-core` to 1,275. The second
  per-wheel force loop remains open.
  G03 second-loop ratio gate and residual saturation: the second loop gates
  on `|dot * 4 - speed| < 0x10000` (pseudo-absolute, so `i32::MIN` passes).
  The residual is reduced by the force only when they share sign and the
  force is smaller in magnitude, zeroed otherwise when the force is not
  smaller, and left unchanged on opposite signs or zeros. The gear-ratio gate
  zeroes the residual when `ctl+0x2ec == 0` and the ratio is zero or has the
  opposite sign. `cmr-core::drivetrain` gains `wheel_ratio_within_unit`,
  `residual_with_saturation`, and `gate_residual_by_gear_ratio`. Three focused
  tests bring `cmr-core` to 1,278. The remaining normalization stages remain
  open.
  G03 second-loop residual computation and ratio gate: the second residual
  is `-qmul(qmul(ratio, 0x4000), DAT_0050c5c4)` with an optional `0x147a`
  factor when the per-wheel flag is set; the third residual is
  `-qmul(dot, DAT_0050c5c4)`. Negation and multiplication wrap. The
  normalization stage is selected when `|residual| > 0x10000` (pseudo-absolute,
  so `i32::MIN` is not large). `cmr-core::drivetrain` gains
  `second_loop_residual`, `second_loop_axis_residual`, and
  `second_loop_ratio_is_large`. Three focused tests bring `cmr-core` to 1,281.
  The remaining normalization stages remain open.
  G03 second-loop large-ratio normalization: the large-ratio branch computes
  `normalizer = |local_30|` and `reciprocal = 2^32 / normalizer` (signed
  division, `-1` when zero), scales and sums the two direction sources by
  `qmul(local_2c, reciprocal)` and `qmul(local_30, reciprocal)`, and normalizes
  the sum with the legacy generated-table rule (zero square sum yields zero
  direction). The length is `qmul(square_sum, normalizer)`.
  `cmr-core::drivetrain::normalize_second_loop_direction` reproduces it. Three
  focused tests bring `cmr-core` to 1,284. The small-ratio path and the
  remaining force-pass stages remain open.
  G03 second-loop small-ratio branch selection: the normalizer is `|local_30|`
  when `|local_2c| < 0x10001` or `|local_2c| < |local_30|`, and `|local_2c|`
  otherwise. Both branches scale and sum the direction sources, normalize with
  the legacy generated-table rule, and compute `length = qmul(square_sum,
  normalizer)`. Two additional focused tests bring `cmr-core` to 1,286. The
  post-normalization force-pass stages remain open.
  G03 post-normalization arctan index fold: a zero product yields `0`, a zero
  denominator with a nonzero product yields `0x400`; otherwise the ratio is
  folded into the first octant (`min/max`, never above `1.0`), indexed by
  `(numerator << 16) / denominator >> 7`, complemented (`0x400 - index`) when
  the original ratio exceeded `1.0`, and negated on opposite signs.
  `cmr-core::drivetrain::arctan_ratio_index` reproduces it with the
  runtime-built arctan table injected. Two focused tests bring `cmr-core` to
  1,288. The cosine modulation and remaining force accumulation remain open.
  G03 second-loop gated accumulation factor: produced only when the speed
  delta and gear ratio share a nonzero sign, `qmul(qmul(delta, 0x4000),
  qmul(DAT_0050c5c4, 0x1999))`, with the result replaced by the signed limit
  when the per-wheel limit is below its magnitude.
  `accumulate_direction_force` reproduces the wrapping
  `accumulator += qmul(component, factor)` accumulation. Two focused tests
  bring `cmr-core` to 1,290. The table-driven final accumulation and the
  force-pass head remain open.
  G03 final table-driven accumulation factor: no factor when the direction
  dot is zero; otherwise `dot >> 2` scaled by `qmul(DAT_0050c5c4,
  table_factor)` when the table entry is present, or `qmul(DAT_0050c5c4,
  0x8000)` otherwise, then negated with wrapping.
  `final_loop_accumulation_factor` reproduces it. One focused test brings
  `cmr-core` to 1,291. The force-pass head computations remain open.
  G03 force-pass head projection-normalization:
  `rejected = state - normal * dot(state, normal)` with wrapping Q16.16
  arithmetic, then the legacy generated-table normalization (zero square sum
  yields a zero vector). The normal is the AK11 final vector at
  `+0x324/+0x328/+0x32c`; the two state vectors are at `+0x550/+0x554/+0x558`
  and `+0x544/+0x548/+0x54c`. `cmr-core::drivetrain::project_and_normalize`
  reproduces it. Two focused tests bring `cmr-core` to 1,293. The first-loop
  drive factor and remaining head computations remain open.
  G03 force-pass head cross-projection: `cross = v x u` with exact
  right-handed Q16.16 products and wrapping subtractions, then projected
  against the AK11 normal with the same rejection and legacy normalization.
  `cmr-core::drivetrain` gains `cross_q16` and `cross_and_project_normalize`.
  Two focused tests bring `cmr-core` to 1,295. The config-gated vector
  selection and the first-loop drive factor remain open.
  G03 drive factors and pair orthogonalization: the force-pass head computes
  `DAT_0050c5c4 = qmul(state+0x52c, DAT_004eac70)`, `DAT_0050c600 =
  qmul(DAT_004eac70, 0xf000)`, and `DAT_0050c5c0 = qmul(DAT_004eac6c,
  0x11113)`. Its alternate branch orthogonalizes a vector pair:
  `scaled_dot = qmul(qdot(v, w), 0x20000)`, `rejected = w - v * scaled_dot`,
  then `result = v - rejected * qdot(v, rejected)`, normalized with the legacy
  generated-table rule. `cmr-core::drivetrain` gains `compute_drive_factors`
  and `orthonormalize_pair`. Four focused tests bring `cmr-core` to 1,298. The
  config-gated vector selection remains open.
  G03 config-gated vector selection and basis stores: the alternate
  pair-orthogonalization branch runs when `DAT_00d3ca80+0x26 & 0x80` is set,
  or when `DAT_00d3ca88 == 0` and `FUN_0042a840()` returns nonzero (the helper
  is not called when `DAT_00d3ca88 != 0`). Otherwise the projection results
  replace the initial direction sources only when `DAT_00d3ca80+0x25 & 0x10`
  is set. `select_head_vector_path` reproduces the branch selection, and
  `HEAD_BASIS_STORE_OFFSETS` pins the four three-dword basis rows at `0x380`,
  `0x38c`, `0x398`, and `0x3a4`. Two focused tests bring `cmr-core` to 1,300.
  The force-pass head and both per-wheel loops are now contracted; runtime
  sim-input wiring is next.
  G03 first per-wheel iteration composition: `execute_first_wheel_iteration`
  applies the drive, slip candidate and sign-disagreement, the slip gate and
  clamped/downshifted slip force (or a zeroed redrive), wheel-speed
  integration, and the zeroing gate in native order. Three focused tests cover
  the full slip path, the closed-gate redrive and speed flip, and the rear
  handbrake-off slip path, bringing `cmr-core` to 1,303. The recovered rules
  are now a coherent per-wheel sim step with injected runtime inputs; the
  second-loop composition and real car-data tables remain open.
  G03 second-loop cosine modulation:
  `modulated = qmul(coefficient, angle_cos)`, then
  `root = legacy_sqrt(qmul(angle_cos, angle_cos) + qmul(modulated, modulated))`,
  and finally `qmul(root, config_scale)`, with the cosine supplied by the
  injected runtime-built table. `cmr-core::drivetrain::cosine_modulated_component`
  reproduces it. One focused test brings `cmr-core` to 1,304. The second-loop
  composition and runtime table wiring remain open.
  G03 second-wheel core composition: `execute_second_wheel_core` applies the
  ratio and residuals, residual saturation and gear-ratio gate, the two
  residuals, direction normalization, arctan folding, and cosine modulation in
  native order. The independent re-derivation test checks every stage, bringing
  `cmr-core` to 1,305. The full second-loop accumulation and runtime table
  wiring remain open.
  G03 full second-loop accumulation: the `ctl+0x420`-gated factor
  (sign-matched speed delta and gear ratio, scaled and clamped) and the final
  table-driven factor (`dot >> 2` scaled by the table factor or `0x8000`,
  negated), both accumulating `qmul(component, factor)` into the three force
  accumulators with wrapping addition, using the head's first direction
  source. The independent re-derivation test additionally checks both factors
  and the exact accumulator values, bringing `cmr-core` to 1,305. Runtime
  table wiring is next.
  G03 axle factor and travel update: the front/rear factors are
  `qmul(car_type_constant, travel)` (front `DAT_0050e19c`, rear `DAT_0050e1a0`,
  travel `DAT_0050e184`), and the travel update decrements by
  `min(0x10000, qmul(excess, 0x8000))` on excess, bumps `DAT_0050e160` by
  `min(0x10000, qmul(excess, 0xccc))`, then applies the surface bump
  (`0x147`/`0x20` by the surface flag), capped at `0x10000` with the flag
  cleared. `cmr-core::drivetrain` gains `derive_axle_factor`,
  `update_travel_on_excess`, `apply_surface_bump`, and `surface_bump_value`.
  Three focused tests bring `cmr-core` to 1,308. The per-car-type loader's
  constant sources are next.
  G03 car-type runtime defaults: `DAT_0050e19c = 0x10000` (front),
  `DAT_0050e1a0 = 0x10000` (rear), `DAT_0050e184 = 0x10000` (initial travel),
  and `DAT_0050e18c = 0` (per-wheel constant), all written by `FUN_0041e3f0`
  for every car type. With these defaults the axle factors are simply the
  travel value and the wheel drive adds no per-wheel bias.
  `default_car_type_constants` returns them. Two focused tests bring
  `cmr-core` to 1,309. The engine/gear and per-surface capability tables
  remain to be sourced.
  G03 gear ratio and auxiliary value derivation:
  `ratio = qmul(original_at_0x40, DAT_0050c860)` and
  `value = qmul(original_at_0x60, reciprocal)`, with the reciprocal
  `0x100000000 / iVar12` or `-1` when `iVar12 == 0`. The original ratio values
  are runtime car data. `cmr-core::drivetrain` gains `derive_gear_ratio`,
  `derive_gear_aux_value`, and `reciprocal_or_minus_one`. One focused test
  brings `cmr-core` to 1,310. The original ratio tables' loader and the
  per-surface capability tables remain to be sourced.
  G03 gear ratio wiring from control families: `raw_copy` (block `0x210`)
  feeds the final ratio with `DAT_0050c860`, and `reciprocal_copy` (block
  `0x230`) feeds the auxiliary value with the reciprocal. Those blocks come
  from the recovered initializer's control-family stages, so the gear ratio
  wiring is closed without an external loader.
  `derive_gear_ratio_and_aux_from_families` composes them. One end-to-end test
  brings `cmr-core` to 1,311. The per-surface capability tables remain to be
  sourced.
  G03 per-surface capability enable counters: `FUN_00424a40` marks enabled
  surface ids, then accumulates each enabled id's per-surface capability byte
  into the corresponding enable counter (16 counters over 120 surface ids,
  duplicate ids enabled once). `accumulate_effect_enables` reproduces it with
  a safe-contract out-of-range rejection. Two focused tests bring `cmr-core`
  to 1,313. The per-surface capability tables are game data (not vendored).
  G03 reviewed-store initializer composition: the integration test opens a
  synthetic CMPR-style reviewed store, constructs `CompiledDirectCaseSet`, and
  runs the complete composed initializer. It independently checks the selected
  direct state/control case and derives the gear ratio and auxiliary value from
  the resulting control families. One focused test brings `cmr-core` to 1,314.
  The reviewed store and runtime inputs are synthetic fixtures, not a live
  car-data run; per-surface capability tables remain caller-supplied game data.
  G03 surface-capability provenance: fresh `FUN_00424a40` pseudocode identifies
  sixteen contiguous PE-backed 120-byte tables at `0x4ec0f8`, while the fifth
  enable counter reads runtime-backed `DAT_0054f824` outside that span.
  `extract_official_eu_surface_capabilities` recovers the PE-backed tables in
  address order through the checked PE32 mapping, without embedding values.
  Two tests bring `cmr-formats` to 160 library tests. Reviewed import/store and
  application wiring remain separate; the runtime fifth table is not extracted.
  G03 reviewed capability import: the official-EU profile binds the 16-by-120
  unsigned-byte shape and exact executable identity.
  `cmr-assets extract-surface-capabilities` authenticates and atomically writes
  the 1,920 user-derived bytes plus path-free state under an explicit ignored
  output root. Two library tests and one CLI test bring `cmr-assets` to 78
  library and nine binary tests; a real authenticated import passed without
  exposing values. The immutable store/counter wiring and runtime fifth table
  remain separate.
  G03 capability store and counter wiring: `SurfaceCapabilityStore` revalidates
  the import before owning all sixteen address-ordered tables without retaining
  paths. `SURFACE_EFFECT_PE_TABLE_INDICES` and
  `accumulate_effect_enables_from_extracted` map fifteen PE tables plus the
  injected runtime fifth table into the sixteen counters; the trailing PE table
  is not consumed by `FUN_00424a40`. Two store tests bring `cmr-assets` to 80
  library tests and one mapping test brings `cmr-core` to 1,315. The opt-in
  official extraction/reopen smoke passes. Runtime production and application
  wiring remain open.
  G03 runtime fifth-table startup boundary: all three literal address sites are
  reads, no direct writer is analyzed, and no file mapping exists for
  `DAT_0054f824`. Its loader-established value is zero, exposed as
  `ZERO_INITIALIZED_RUNTIME_FIFTH_CAPABILITY_TABLE`; one test brings `cmr-core`
  to 1,316. Indirect runtime writes remain unknown and application wiring is
  separate.
  G03 application session surface inputs: `cmr-app::SessionSurfaceInputs` owns
  the reviewed store and zero-initialized runtime table, and computes counters
  only for caller-supplied stage IDs. Two tests cover the full map, invalid IDs,
  and tamper rejection, bringing `cmr-app` to 23 library tests; the opt-in
  official session-open smoke passes. Stage-ID production and weather/render
  consumption remain open.
  G03 stage `.srf` production: `FUN_00480a10` passes the shared loader's decoded
  `.srf` allocation and byte count directly to `FUN_00424a40`.
  `SessionSurfaceInputs::open_stage` loads those bytes through the bounded CMPR
  decoder and `stage_effect_enables` consumes them. One test brings `cmr-app`
  to 24 library tests; real AU1 passes. Weather/render consumption remains open.
  G03 stage weather gating: `StageWeatherEffects` maps the recovered splash (0),
  dust (2), and snow (5) counters to booleans.
  `SessionSurfaceInputs::stage_weather_effects` composes them from the `.srf`
  bytes and reviewed tables. One core test brings `cmr-core` to 1,317; app
  assertions and real AU1 pass. Other counters remain unnamed and actual
  particle/render spawning is separate.
  E10 weather-gated particle dispatch: `StageWeatherEffects::allows` and
  `gated_particle_effect` suppress dust, snow/snow-cloud, and splash when their
  counters are disabled, leaving clods, leaves, and gravel unaffected.
  `SessionSurfaceInputs::stage_particle_effect` composes the same path. One test
  brings `cmr-core` to 1,318; app assertions cover composition and invalid IDs.
  Actual particle pool/render spawning remains separate.
  E10 particle-pool records: `ParticleRecord`, `ParticlePool`, and
  `ParticlePools` reproduce the recovered 80-byte field writes, type-selected
  sprite/tuning values, frame/flags and 100-slot wrapping across eight pools.
  `stage_spawn_particle` applies weather gates before record creation. Two tests
  bring `cmr-core` to 1,320; app coverage checks creation and suppression.
  Unknown fields use neutral offset names and render traversal remains separate.
  E10 common pool update: `advance_common` reproduces the active-record position
  and `0xf333` velocity integration, selected vertical/lifetime decrements,
  `word_38` advance, age increment and deactivation rules from `FUN_00427d20`.
  Respawn preserves `word_38`. Two tests bring `cmr-core` to 1,322. Type-specific
  physics and final sprite rendering remain separate.
  E10 type-specific updates: `particle_clod_polynomial` and
  `advance_type_specific` reproduce the clod ground-relative
  motion/reset/deactivation and splash damping/random adjustment before the
  common update. Deactivation clears the recovered fields and Y sentinel. Two
  tests bring `cmr-core` to 1,324. Final sprite rendering remains separate.
  E10 sprite render queue: `SpriteRenderQueue` reproduces the recovered
  resource/depth/sequence append contract and its 100-entry cap. One test brings
  `cmr-core` to 1,325. Pool-field depth mapping and final draw submission remain
  separate.
  E10 active-particle render composition: `ParticlePool::render_queue` emits only
  active records with camera-relative fixed-point depth using the recovered
  square-root approximation. One test checks that approximation and excludes a
  deactivated record, bringing `cmr-core` to 1,326. Final draw submission
  remains separate.
  E10 final sprite submission: `SpriteDrawSubmissions` reproduces the 84-byte
  `FUN_004a4330` record layout, including packet/resource fields, overlapping
  kind bytes and tag, plus the 600th rejection. One byte-for-byte test brings
  `cmr-core` to 1,327. Backend submission remains behind this contract.
  See
  `rea.official-eu.sim-compiled-car-initializer-schema`,
  `rea.official-eu.sim-compiled-control-run-transform`,
  `rea.official-eu.sim-compiled-center-split`,
  `rea.official-eu.sim-compiled-aux-control-patch`,
  `rea.official-eu.sim-compiled-config-options`,
  `impl.sim-compiled-car-initializer-schema`,
  `impl.sim-compiled-car-initializer-extractor`,
  `impl.sim-compiled-car-initializer-store`,
  `impl.sim-compiled-car-initializer-consumer`,
  `impl.sim-compiled-control-run-transform`,
  `impl.sim-compiled-center-split`,
  `impl.sim-compiled-aux-control-patch`,
  `impl.sim-compiled-aux-control-extractor`,
  `impl.sim-compiled-aux-control-store`,
  `impl.sim-compiled-aux-control-consumer`,
  `impl.sim-compiled-config-options`,
  `rea.official-eu.sim-compiled-stage-dataflow`,
  `impl.sim-compiled-stage-effects`,
  `impl.sim-compiled-common-prefix`,
  `rea.official-eu.sim-compiled-post-helper-initialization`,
  `impl.sim-compiled-post-helper-initialization`,
  `rea.official-eu.sim-compiled-state-758-selection`,
  `impl.sim-compiled-state-758-selection`,
  `rea.official-eu.sim-compiled-control-product-pair`,
  `impl.sim-compiled-control-product-pair`,
  `rea.official-eu.sim-compiled-initializer-suffix-map`,
  `rea.official-eu.sim-compiled-suffix-defaults`,
  `impl.sim-compiled-suffix-defaults`,
  `rea.official-eu.sim-compiled-state-reciprocal-trio`,
  `impl.sim-compiled-state-reciprocal-trio`,
  `rea.official-eu.sim-compiled-control-clear-loop`,
  `impl.sim-compiled-control-clear-loop`,
  `rea.official-eu.sim-compiled-half-sign-mixed-reset`, and
  `impl.sim-compiled-half-sign-mixed-reset`,
  `rea.official-eu.sim-compiled-call-spine`, and
  `impl.sim-compiled-call-spine-plan`,
  `rea.official-eu.sim-compiled-state-basis-pair`, and
  `impl.sim-compiled-state-basis-pair`,
  `rea.official-eu.sim-compiled-eight-lane-caller`, and
  `impl.sim-compiled-eight-lane-postprocess`,
  `rea.official-eu.sim-compiled-lane-dispatcher`, and
  `impl.sim-compiled-lane-dispatcher`,
  `rea.official-eu.sim-compiled-lane-build`, and
  `impl.sim-compiled-lane-build`,
  `rea.official-eu.sim-compiled-lane-candidate-chooser`, and
  `impl.sim-compiled-lane-candidate-chooser`,
  `rea.official-eu.sim-compiled-lane-spatial-hierarchy`, and
  `impl.sim-compiled-lane-spatial-hierarchy`, and
  `impl.sim-compiled-lane-spatial-search`,
  `rea.official-eu.sim-compiled-lane-validator`, and
  `impl.sim-compiled-lane-validator`,
  `rea.official-eu.sim-compiled-lane-decoder`, and
  `impl.sim-compiled-lane-decoder`,
  `impl.sim-compiled-lane-geometry-provider`, and
  `impl.sim-compiled-eight-lane-executor`,
  `rea.official-eu.sim-hpc-geometry-loader`, and
  `impl.sim-hpc-geometry-parser`, and
  `impl.sim-hpc-geometry-runtime`,
  `rea.official-eu.sim-post-lane-global-normal`, and
  `impl.sim-post-lane-global-normal`, and
  `impl.sim-eight-lane-post-barrier`.
  See evidence
  `rea.official-eu.sim-drivetrain`.
- The grip/surface response and velocity integration run as later passes of the
  same `0x407690` solver chain. The lateral-grip constraint `0x41b270` strips the
  sideways velocity component each step (`v -= (v - fwd*dot(fwd,v))*K`, `K`~0.02
  normal / ~0.01 handbrake); the slope-grip factor `0x408f50` ramps from zero at
  normal-Y ~0.70 to a ~0.90 flat-ground ceiling; the integrator `0x40a4a0` computes
  `accel = force*invMass + gravity`, quadratic drag `v*speed`, and
  `v += accel*0.2` (the 40 ms step folded into the gain). Per-wheel surface id
  `ctrl[0x42a+k*2]` indexes 8-byte grip records with ~0.24 (high) / ~0.15 (low)
  tiers. Reconstructed in `cmr-core::vehicle_dynamics` over a new Q16.16
  `cmr-core::fixed_point::Vec3`. `0x40d810` now distinguishes raw wheel speeds
  at `+0x2a8[4]` from smoothed/current speeds at `+0x2b8[4]`. Its signed
  compare/select at `0x40f5e0..0x40f5e8` is reconstructed as
  `select_signed_force_candidate` without assigning physical semantics. A
  separate helper reconstructs both lookup-free candidate branches.
  `derive_lookup_dependent_candidates` now reconstructs the remaining combined
  branch, using `signed_ratio_angle` plus exact source-owned trig and legacy
  square-root lookups. A nonzero
  `+0x420` admits the
  recovered same-sign wheel correction; its modular candidate, signed
  threshold/alternate, and remaining-budget rules are reconstructed as three
  pure helpers. The share/ramp/gain scalar at the core of `FUN_0040fbe0` is
  reconstructed with validated one-to-four-lane counts and its exact upper-only
  clamp. The adjacent surface-indexed coefficient/override gates and scalar are
  also reconstructed. `apply_geometric_lane_postprocess` now implements the
  full two-pass geometric contract in `FUN_0040fbe0`: four neutral A/B/C vector
  lanes, strict angle/Y/residual-dot gates, one-to-four-lane gain sharing,
  generated arcsine plus legacy-square-root normalization, signed
  wrapping-absolute alignment bounding (including `INT_MIN` passthrough), and
  wrapping adds into `+0x45c+12*lane`. The only fallible domain is the unsafe
  arcsine `INT_MIN` lookup; the API stages all four decisions and contributions
  before publication, so that lookup error leaves every accumulator unchanged.
  `apply_lane_accumulation` composes the following per-lane tail: overwrite the
  accumulator by the selected scalar, optionally add the first direction,
  derive the second scalar from a projection against the already-updated
  accumulator and optionally add the second direction, then independently
  project the surface vector and add its scalar along the first direction.
  Eight focused tail tests brought `vehicle_dynamics` to 63 tests. The pure API
  returns the resulting value but does not claim the shipped mutations were
  atomic. The upstream direction producer is now reconstructed as
  `construct_lane_directions`: source vectors at physics `+0x550`, `+0x544`,
  and `+0x5d4` are rejected from axis `+0x324` and normalized through the
  legacy square-root/reciprocal path, while the fourth begins with the raw
  ordered cross `cross(+0x5d4,+0x264)` before rejection. Baseline packing is
  A=`[P,P,S,S]`, B=`[R,R,T,T]`; the four validity flags are snapshots of the
  initial A square sums and do not follow later exchanges. Neutral modes expose
  Preserve (baseline), Exchange (A=`[S,S,P,P]`, B=`[T,T,R,R]`), and Rebuild
  (A=`[P,P,C,C]`, B=`[R,R,U,U]`). Rebuild forms raw
  `C=P-T*qmul(dot(T,P),0x20000)` without normalizing C, then normalizes
  `U=T-C*dot(T,C)`. Published unique vectors preserve control snapshot order
  `+0x380,+0x38c,+0x398,+0x3a4`. Eight focused direction tests bring
  `vehicle_dynamics` to 71 tests; `legacy_math` remains at 13. The callback and
  global conditions that resolve a mode remain caller-owned and unimplemented,
  and semantic vector/mode names remain unknown. Fixed-point wrap/truncation
  means no exact unit/orthogonal claim is made. The translational portion of
  `FUN_0040a4a0` is now also reconstructed by `apply_translation_step`:
  position is physics `+0xc8..+0xd0`, velocity `+0xd4..+0xdc`, prior velocity
  `+0xe0..+0xe8`, and cached prior speed `+0xec`. The old cached speed scales
  velocity before force/inverse-mass/additive-acceleration processing; then
  `dv=qmul(acceleration,0x3333)`, velocity advances, and position adds
  `new_velocity-qmul(dv,0x8000)`. This corrects the older `+0x3cc` velocity and
  `+0x3d8` position labels: those are respectively an additive
  acceleration-like term and part of the rotational/moment chain. Nine
  focused tests, including final-position wrapping, bring `vehicle_dynamics`
  to 80. “Translational damping” is a
  value-flow description only; a distinct suspension damper, mode semantics,
  and the caller resolver remain open. The table-provenance pass proves there
  is no single runtime table loader: `DAT_004f72cc` is the first of two
  adjacent PE-backed 120-word signed coefficient families, while
  `FUN_00404440` materializes car/gear tuning through fourteen compiled cases
  and `DAT_0050e170` is BSS/runtime geometry state. `cmr-formats::pe_surface`
  bounds-checks and extracts only the two official-EU PE families; the
  profile-gated `cmr-assets extract-surface-tables` command verifies the
  user-owned executable before atomically writing 960 bytes plus path-free
  metadata under an explicit ignored runtime-data destination.
  `cmr-assets::SurfaceTableStore` now revalidates the state schema, reviewed
  profile ID and exact digest, source metadata, output path/size/hash, and the
  real 960-byte data file before owning both decoded families without retaining
  a path. Its persistent-symlink rejection assumes the caller keeps the
  directory stable during `open`; concurrent adversarial mutation is outside
  the contract. The checked core adapter accepts the first `[i32; 120]`
  family, validates the index before override handling, preserves the raw
  signed Q16.16 word, and delegates to the recovered lane-scalar rule. Five
  adapter tests bring `vehicle_dynamics` to 85; `cmr-core` has 1,095 tests and
  `cmr-assets` has 50 library plus three CLI tests. The second family remains
  store-only with unknown semantics, and no application/session wiring is
  claimed. These families remain distinct from FFB `DAT_004edb70`, compiled
  car initializer `FUN_00404440`, and BSS `DAT_0050e170`. The compiled
  initializer's direct cases now have the separate verified extractor/store
  described above; their gameplay consumer, omitted common path, and the BSS
  consumers remain open.
  `signed_arcsine_phase` preserves the signed lookup, `0x10000` clamp,
  index-4096 adjacent angle-table zero, and rejects unsafe `INT_MIN` instead of
  reproducing the unchecked read. `signed_ratio_angle` preserves the observed asymmetric
  zero, sign/swap, and index-512 adjacent-zero rules. Its zero gates precede
  validation: `(0,INT_MIN)=0`, `(INT_MIN,0)=0x400`, and `INT_MIN` is rejected
  only when both operands are nonzero;
  `0x400` is a quarter-period and `0x1000` the full phase, not an atan2 claim.
  See evidence
  `rea.official-eu.sim-grip`,
  `rea.official-eu.sim-signed-force-candidate-select`,
  `rea.official-eu.sim-force-candidate-fast-paths`,
  `rea.official-eu.sim-signed-ratio-angle`,
  `rea.official-eu.sim-lookup-dependent-candidates`,
  `rea.official-eu.sim-geometric-lane-postprocess`,
  `rea.official-eu.sim-wheel-correction-arithmetic`,
  `rea.official-eu.sim-postprocess-correction-scalar`,
  `rea.official-eu.sim-surface-indexed-lane-scalar`,
  `rea.official-eu.sim-runtime-math-tables`, and
  `impl.legacy-math-tables`, `rea.official-eu.sim-runtime-table-provenance`,
  `impl.sim-surface-table-extractor`, and `impl.sim-surface-table-store`.
- Static scenery collision runs before per-car mechanics through
  `0x43d5d0 -> 0x43e640 -> 0x43d760`; the later `0x410a60` step delegates the
  crash path to `0x4115d0` once the out-of-control flag `car+0x680` is set.
  Linear impulse accumulates at `car+0x3cc..`, angular impulse at `+0x3d8..`,
  and `+0x3f0..` is a mutable local contact/lever arm rather than angular state
  or a contact normal. The normal is published separately by the narrow phase. The
  linear impulse magnitude feeds the impact
  computer `0x41f630` (threshold `0x2624`, slope `0xf536`, clamp 1.0), and the
  same 0..1 impact drives the pitched impact sound (`0x469e80`), force feedback,
  and damage. Damage is tracked twice: a coarse crash counter `car+0x678` (+1 per
  contacting tick, clamp 100, totalled latch `car+0x67c`) and a fine per-region
  mesh accumulator `DAT_0050e0d8` (`w = (1-dist^2)*impact`, clamp 1.0), with body
  panels shed at region-damage `0x3333` / `0x6666` (`0x44a770`). Crash reactions
  (`0x4756f0`) pick a sound bank by severity (tier-1 max `0x4ccc`) and material
  (5=METAL/8=HAY_BALE/0xd=PLASTIC/0x10=RUBBER). Out-of-bounds is handled by
  node-keyed stage trigger volumes (`0x403a00` on `car+0x750`): type 6 kills
  (`car+0x67c`), 0x15 arms the `car+0x75c` countdown, 7..=0xB switch surfaces.
  Reconstructed the impact, crash counter, deform/region accumulate, damage
  level, materials, and trigger types in `cmr-core::collision` (with
  `Fixed16::sqrt`/`Vec3::length` added to `fixed_point`).
  `cmr-core::static_world_collision` now owns the tested 4,096-cell ordered
  provider boundary, current/previous five-ID history, raw material propagation
  and distinct linear/angular/local-contact-arm effects. Static recovery pins
  exact OBJ OBB geometry/projection, main-C3D grid derivation, prepend order,
  literal-zero history identity and both reciprocal-corner narrow paths;
  `static_collision_response` pins the direct normal response and exceptional
  branches. Parsed scene construction and the pure live-car hull/projection/
  primary/neighbor-cell producer are now implemented below; authenticated
  response inputs and live session integration remain open, and renderer bounds are
  not substituted. See
  evidence `rea.official-eu.sim-collision` and
  `impl.static-world-collision-orchestration`.
  `static_collision_scene_builder` now bridges checked main/OBJ C3D parsers to
  those records using the recovered admission, shallow traversal, immediate-
  parent translation, root-local scale/basis and native-prepend cell rules.
  AU1 read-in-place diagnostics account for 2,169 records in 156 occupied
  cells. `live_static_collision_car_frame` preserves the pre-integration X/Z,
  exact eight-point/deformation arithmetic, projection cache and strict neighbor
  ordering. `static_collision_kernel` composes the normal response with one
  signed tick history while keeping alternate overlap distinct. The app retains
  the parsed scene. The exact primary selector scans retained authored XYZ
  centers with wrapping distance arithmetic and strict first-wins ties;
  compiled setup now publishes collision halves, AK11 axis, corrected basis and
  typed deformation scalars. `static_collision_response_input` now replaces
  arbitrary response thresholds with the per-contact final-normal/limit
  producer, while response derives exact accumulated-correction damage severity.
  Canonical contact axes are now reconstructed from their literal normalized
  seed, with strict damage-branch entry separate from scalar severity. Live
  contact count `+0x674` is now published for the authenticated setup snapshot
  from exact lane/hull owners. Pending `+0x3cc`, single-owner angular `+0x3d8`,
  history, persistent `+0x680/+0x688` flags and deformation scalars now have
  typed live-car ownership, and translation consumes/clears prior linear
  correction once. A typed lazy damage owner now supports an unknown number of
  ordered normal hits without a production replay FIFO. Per-tick republishing, the
  concrete damage-object implementation, live
  basis/scalar evolution and mechanics-session publication remain open.
- Stage timing uses the global clock `DAT_005ffc00`, a Q16.16 value in seconds
  advanced once per physics tick (step `2^32/(rate*0x10000)` = `0xa3d`~0.04 s at
  25 Hz, the same dt as `sim_clock`; `0x51e` at 50 Hz), saturating at
  `0xe100000` = 3600.0 s. It is a deterministic tick accumulator, not a
  `timeGetTime` wall clock, so a reset or off-course costs real time implicitly.
  A per-car start timestamp is latched once (`0x47fc50`); elapsed time is read as
  whole seconds (`(now-start)>>16`, `0x47fcf0`) or centiseconds
  (`sec*100 + frac*100>>16`, `0x47fd20` - the stored/displayed unit). Checkpoints
  are type-0 trigger volumes de-duplicated per car by a passed-bitmask
  (`volume+0x2e |= 1<<(car&0x1f)`, `0x403ee0`) that also enqueues the pacenote
  callout; finish (type 1) freezes the player time `DAT_00573a80` and per-car
  time `DAT_005ffbf0[car]` and moves the FSM from in-race state 5 to the
  stage-results state 8. There is **no additive time-penalty system** (no
  false-start/jump/missed-checkpoint/off-course surcharge; the clock never
  pauses) - a recorded negative result. Reconstructed the clock step/advance/cap,
  the seconds/centisecond readouts, the start latch, the checkpoint passed-bits,
  and the finish constants in `cmr-core::stage_timing`. The reference-split
  comparison, the best-time persistence, and the per-driver results record
  remain. See evidence `rea.official-eu.sim-stage-timing`.
- Co-driver call scheduling has two queues: an 8-entry audio ring
  (`DAT_005590c8`, one packed note-id per entry, head/tail mod 8) and a depth-2
  interleaved visual FIFO (`DAT_004f2120`) that drives the on-screen pacenote
  strip (`DAT_0057c178`, stride 0x14, 0x12/track). Submit (`0x444380`, called
  only from the trigger walker `0x403a00` on a type-0 volume, de-duplicated per
  player) optionally swaps the direction modifier (bits 7-8, mirror stages), then
  enqueues the whole id into the ring (blind, no full check) or routes it to the
  visual FIFO. The sequencer (`0x444450`, once per frame before Mechanics, gated
  by `QSWaveMixIsChannelDone` on the single shared speech channel) plays one
  bitfield group per tick - lowest field first via the `audio_pacenote`
  field table - clearing it and writing the remainder back, advancing only when
  the entry hits 0; a following word field (`& 0x78`) inserts the INTO link
  (slot 0x27) for the "corner-A into corner-B" cadence. Scheduling is
  event-driven off checkpoint crossings and channel-free status, with no
  clock/distance sampling (look-ahead is authored into trigger placement); the
  resolved `9*(mod-1)` is a display-row bump, not an audio stride. Reconstructed
  the ring, the incremental sequencer, the modifier swap, the into-link test, the
  routing, and the display arithmetic in `cmr-core::pacenote_schedule`. The
  display-record layout and the mode-gate semantics remain. See evidence
  `rea.official-eu.sim-pacenote-schedule`.
- There is no on-track AI opponent and no single-player ghost: the mechanics
  loop simulates exactly `DAT_00550e90` cars, which the race setup (`0x481350`)
  never sets above 2, and the only second car is a second *human* (split-screen
  `0x485bb0` or network `0x401060`; replay forces 1). No code steers a second car
  from a racing line - the node array `DAT_0057ae6c` is a timing spline read to
  record times (G05), not a driving line. Rival WRC drivers are a name roster
  (`0x507300`, 8 records) feeding the standings UI; their times are authored data
  outside the executable. Car-vs-car collision (`0x442740` -> `0x441f30`) runs
  only when two cars are present, not in replay, and split or network.
  Reconstructed the participant-count model, the dispatcher count, and the
  collision gate in `cmr-core::race_participants` (record strides as format
  metadata; name strings not vendored). See evidence
  `rea.official-eu.sim-race-participants`.
- Replay has two subsystems. The authoritative trace (the "Replay" class,
  `0x46d7b0`) is input-based re-simulation saved to `.SSR` files: a `0x40`-byte
  header, a per-frame `0x151ec`-byte initial-state seed, then `nTicks` (7500 at
  25 Hz = 300 s) 4-byte packed control records (actuators quantized `>>10` into
  6-bit fields, XOR-delta coded, plus gear/flags), followed by a two-byte field
  whose consumer and semantic name remain unresolved. Playback decodes the inputs
  back into the control struct and re-runs the normal physics (`0x43afd0`), with
  collision suppressed and the `0x462ac0` tracker used under the playback flag;
  the live frame driver `0x437690` bails and `0x437ef0` drives instead. Files go
  through the shared loader `0x495ed0`, whose optional archive container carries
  the `"CMPR"` magic + size. The same per-car input stream serves the local
  action replay, the `.SSR` shadow, and the network replay car. A separate
  render snapshot buffer (`0x46c7d0`, gated by the record flag) captures 24-byte
  pos+compressed-orientation keyframes at 6.25 Hz (cap 0x8ca x 6 cars) and
  interpolates them into the render transforms for smooth display only - it
  never feeds physics. Reconstructed the object sizing/region layout, exact
  four-byte actuator records, digital `-66` decoding, double-XOR current fields,
  repeat counting, CMPR header and snapshot math in `cmr-core::replay`. The
  vertical trace now uses those native records and rejects partial input. Region A
  is sectioned into car-type, control, physics, 500 bounded objects and trailing
  state/timer fields. `.SSR` writing is explicitly unsupported by resolved static
  callers; network replay uses the same four-byte records with native duplicate
  suppression, completing G08. Transport envelopes remain networking work. See
  evidence `rea.official-eu.sim-replay`, `rea.official-eu.replay-control-record`,
  `rea.official-eu.replay-region-a-layout`, and
  `rea.official-eu.network-replay-records`. The auxiliary USA PS1 comparison
  (`psx.usa.replay-object-shape`) corroborates the high-level seed/control/trailing-
  field architecture with edition-specific sizes; it does not establish Windows
  semantics. Co-driver display records now round-trip
  all 20 bytes, the visual FIFO implements rotation, TTL eviction and wrapping
  modifier accumulation, and audio routing uses native replay-car/active-player
  counts, completing G07. `participant_setup_counts` separates replay data slots
  from simulated cars with two-player doubling. `generate_rival_times` recovers
  the procedural tier/RNG/remap/difficulty algorithm without inventing an on-track
  AI, completing G06.
- `cmr-core::race_state` composes the recovered fixed-step pieces (`fixed_point`
  BAM trig + `Vec3`, `vehicle_dynamics` lateral-grip/net-accel/drag/integration,
  and the `stage_timing` stage clock) into a deterministic offline race-state
  trace: one `step` advances a 40 ms / 25 Hz tick in its documented harness order (steer ->
  forward vector -> net force = drive force minus recovered drag -> integrate ->
  recovered half-delta position update -> lateral grip -> advance clock), and all-integer
  `Fixed16` state makes the trace bit-for-bit reproducible. This is a
  composition/determinism harness, not trajectory-accurate: the drive force,
  mass and gravity are explicit parameters because the drive-torque->force
  wiring remains unresolved. The exact translation kernel is now called with
  zero damping after acceleration is computed, preserving the grip order while
  replacing the plain position update. A byte-exact differential fixture is
  still gated on force production. `step_with_first_wheel` now executes the
  recovered first-wheel force pass with explicit runtime inputs and uses its
  force for that translation; a two-tick test covers split, slip clamp, speed
  integration and movement. `step_with_second_wheel` also composes the second
  core from the first-wheel speed delta and explicit vector/table inputs,
  recording residual, angle, component and the selected-scaled lane accumulator
  after both ordered corrections and the independent surface addition. This
  corrects the former zero-initialized accumulator composition. The four-lane
  `step_with_four_wheel_postprocess` now feeds the selected second-core lane and
  the other three runtime lanes through `FUN_0040fbe0`, retaining all four
  accumulators and qualification results. `select_second_wheel_candidate` now
  derives the transition/alternate pair through the appropriate lookup path and
  applies the strict normalization-length selection. The second core now derives
  that scalar from residuals and runtime bases and propagates checked lookup
  failures through race state. The downstream consumer is now identified:
  `sum_translation_lane_forces` sums base and non-wheel forces, then only
  contacted wheel accumulators, before translation damping. The four-lane path
  now performs that sum after postprocessing and translates with the resulting
  world force in the same tick. `apply_contact_vector_update` reconstructs one
  `FUN_0040d400` record with basis projection, legacy normalization, extra-length
  subtraction and the distinct threshold/limit clamp. `sum_contact_moments`
  crosses each transformed force with its contact offset under the native contact
  gates. `transform_contact_force` reproduces the three row-wise `FUN_0048e100`
  dot products, and `transformed_contact_moment` applies that transform before the
  cross product. `apply_angular_momentum_step` scales and combines moments,
  applies componentwise inertia, and negatively integrates angular velocity.
  `clamp_angular_velocity` reproduces the gated signed `0x3333` clamp including
  wrapping-absolute behavior. `rotate_basis_axis_angle` and
  `integrate_orientation` now reproduce the axis-angle matrix, normalization and
  native `1,2,0` update order. `orientation_angle_increment` and
  `integrate_orientation_from_velocities` now connect half-delta angular velocity
  through the single `0x28b` conversion. `apply_control_axis_resets` covers both
  selective angular reset branches and their priority.
  `apply_contact_vector_updates` now traverses all eight records in reverse order
  and preserves skipped destinations. `apply_angular_orientation_pipeline`
  composes momentum, clamp, conversion, selective resets and orientation in native
  order. `suppress_low_speed_position_delta` reproduces the active-contact
  low-speed position gate without changing velocity. `apply_translation_step`
  now applies it after the half-delta calculation, and the four-lane path derives
  activity from contact flags. `step_with_all_four_wheels` runs all first and
  second iterations in native reverse order with axle equalization before the
  geometric postprocess and translation. `drive_stage_with_all_four_wheels`
  completes the same vertical path and reproduces the identical replay outcome,
  with actual prior speed used for low-speed gating.
  `populate_compiled_gear_ratio` maps the selected compiled control-family word
  into both wheel parameter sets without mutating on invalid gears.
  `derive_force_pass_scales` and `derive_alternate_force_pass_factor` reproduce
  the main-head scale products and early factor. Runtime scalars remain injected;
  `populate_force_pass_scales` applies the derived bundle to both wheel parameter
  sets, and the vertical path uses it for every wheel.
  `populate_surface_table_factor` maps the reviewed first coefficient family into
  second-core accumulation with checked indices; an authenticated real store also
  reopens and feeds the adapter. `cmr-app::SessionWheelInputs` exposes the same
  authenticated store to second-core callers. The strict `0xcccc` distance gate,
  count scaling and four response-word timestep products are now reconstructed.
  `build_contact_basis_vectors` reconstructs the twelve primary scalar dwords,
  mirrored negatives, eight-vector layout and velocity translation.
  `populate_contact_basis_vector` selects those vectors into second-wheel surface
  inputs with checked indices and no mutation on rejection; the complete vertical
  fixture derives all four contact inputs through this adapter and retains its
  deterministic finish/replay contract. The complete four-wheel step now rebuilds
  those vectors each tick with lane/indexed validation before any wheel-state
  mutation. `MotionState` now owns the three native basis axes, primary angular
  velocity and compiled half-words, and forms the six angular inputs plus actual
  pre-step velocity without a duplicated template. The compiled initializer's first
  three half-words feed that state directly. After force/translation, the complete
  step applies the recovered angular pipeline and stores basis, angular velocity and
  scaled prior moment back into `MotionState`; the following tick's contact vectors
  therefore use the updated orientation state. The current postprocessed wheel forces
  are transformed through the pre-update basis, crossed with their contact offsets,
  filtered by contact flags, and added to the explicit base moment before that angular
  update. Those offsets now come from the compiled half-sign vectors at native
  `+0x5c/+0x68/+0x74/+0x80`, mapped in wheel-lane order by
  `populate_compiled_contact_offsets`; exact sign and Q16-scaled angular behavior is
  covered. `GameplaySession::new_with_four_wheel` selects this complete producer;
  `try_advance` performs the state-5 driving step before the FSM frame, preserving
  checked errors without advancing the FSM, checkpoint progress or tick count. The
  existing simplified session remains available. `SessionWheelInputs` now constructs
  the complete wheel parameters from reviewed coefficients and compiled state, and
  `run_headless_four_wheel_game_session` drives the FSM through them with checked
  errors. Synthetic authenticated finish/replay and real-store opt-in coverage pass;
  scalar tuning and start-state values remain explicit harness inputs. After lane
  postprocessing and before translation, `apply_contact_vector_updates` now traverses
  all eight records with current contact-basis vectors and persistent skipped-state
  preservation. Their projection axis and rebuilt basis come from compiled post-lane
  outputs, `+0x528` is count-scaled `+0x48`, extra vectors reset, wheel enables follow
  contact flags, and limits/thresholds come from the compiled control-clear loop. Only
  the global update gate and non-wheel enables remain explicit. Every step also runs
  `construct_lane_directions` once and shares its four A/B outputs between second-wheel
  accumulation and the geometric lanes, leaving independent C vectors and active flags
  intact. `resolve_lane_direction_mode` implements the native Rebuild-first selector;
  `derive_lane_direction_pair` copies or rotates S before forming raw R. Two focused
  tests cover selector priority and copy/rotate/cross behavior.
  `stabilize_lane_source_s` now rejects/normalizes S, blends it toward `+0x258` by the
  recovered root/response/timestep factor, normalizes below 1.0 or uses the axis
  verbatim above it. S persists in `MotionState`; contact indices are validated before
  any S mutation. The interactive windowed session uses the same authenticated
  four-wheel constructor and fallible advance, returning checked producer errors after
  the SDL loop stops. Displayed execution remains unverified here. The global update
  gate is computed from the transformed projection axis, and all eight `+0x73a` flags
  plus the `+0x742` failing count come from `derive_compiled_lane_thresholds`; that
  count drives `+0x528`. G09 is complete as a deterministic offline-trace contract:
  the complete producer, checked errors, start/finish/replay and authenticated sessions
  are covered, with explicit harness tuning documented rather than trajectory-equivalent
  behavior. G10 is complete under its documented harness contract: start, drive,
  checkpoint finish, save/replay, complete production and application wiring are all
  covered. `SOLVER_GLOBAL_CONSTANTS` pins the static `DAT_004eac6c/70/74` values to
  `0x10000` and uses them for lane stabilization, angular timestep and force scaling in
  the application path. `populate_derived_second_inputs` now supplies the wheel/state
  and direction-B/contact-vector Q16 dot products before each second-wheel iteration,
  plus the residual-enable flag from `+0x43b` and lane-split `+0x158` checks. The flag
  is independent of slip-sign disagreement; lanes 0/1 and 2/3 use different disabling
  values. `populate_wheel_vector_inputs` now copies each constructed A direction and
  current contact vector into both wheel loops before iteration. The recovered
  arctangent lookup uses `SECOND_WHEEL_ARCTAN_TABLE`, the canonical 512 source-owned
  words plus the native exact-1.0 adjacent zero sentinel. See evidence
  `impl.sim-race-state-trace`, `impl.sim-derived-second-inputs`,
  `impl.sim-wheel-vector-inputs`, `impl.sim-translation-step`, and
  `impl.sim-full-second-accumulation`. `reset_wheel_speed_state` separately models
  the reset/respawn `+0x158` mode writes, including untouched raw/smoothed fields,
  and the unconditional residual-enable value `10` used by application parameters.
  Its scalar and zero application start speeds remain explicit inputs.
  `FourWheelState` now persists the residual countdown and native `state+0xec` speed.
  Each step applies the mode-selected raw-speed threshold before wheel production,
  then updates speed with `sqrt_q16` after translation. The reset value `10` is no
  longer treated as a permanent enable. `smoothed_wheel_speeds` is now separate from
  raw wheel state: second-core execution reads its prior values, and post-translation
  tracking applies the recovered snap/step and `0x7ae` dead-zone rules. The driven
  integration and neutral-gear branches of `FUN_004088c0` remain separate.
  `smoothed_speed_uses_tracking` and `integrate_driven_wheel_speed` now recover the
  driven branch as pure contracts, but are not connected to the unrelated `+0x540`
  force. Native `+0x2c8` force and global scalar production remain open.
  `update_smoothed_wheel_speeds` handles the neutral-gear zero branch before any
  tracking, then resumes ordinary tracking for a nonzero gear ratio.
  `derive_driven_smoothing_scalars` now supplies slip, direction, signed speed limit
  and engine load with the recovered reciprocal/clamp order. Native runtime input
  population and `force_2c8` production remain separate.
  `derive_smoothed_engine_speed` now produces `engine_speed_2e8` from the smoothed
  axle averages and load cap after the wheel update, while neutral copies `engine_2e0`.
  Engine-control and redline/slip/load inputs remain explicit. The ordinary path now
  advances `engine_2e0` through `integrate_engine_from_raw_wheels`, applies the rev
  limiter and stores `limiter_444`/`limiter_cut_448` before smoothing. Neutral/idle
  special cases and `force_2c8` production remain separate.
  `derive_engine_force` and `integrate_idle_engine` now produce `engine_force_530`
  and advance the common neutral/idle branch before rev limiting. `idle_6e8` and
  `idle_countdown_6ec` guard unrecovered special states. `advance_idle_reset` now
  handles the `6e8 != 0` clear/reload/decrement transition before rev limiting;
  the separate nonzero-idle oscillation remains open. `advance_idle_oscillation` now
  covers that branch, including sine/RPM replacement, countdown and optional response
  write. Persistent `response_2d8` is initialized once and shared with lane stabilization
  and engine force.
- `cmr-core::vertical_path::drive_stage` completes the start -> drive -> finish
  -> save-replay vertical path over the deterministic race state: it starts a
  stage, drives a scripted control sequence at the fixed tick (crossing
  checkpoints de-duplicated with `stage_timing`'s passed-bitmask and recording
  each tick's actuator into the replay input trace via the recovered
  quantize/XOR-delta codec), freezes the finish time in centiseconds on crossing
  the line, and verifies that re-running the recorded inputs from the same start
  reproduces the identical outcome - the input-based re-simulation guarantee.
  Harness simplifications (documented): a 1-D +x gate line and only the
  forward-force channel serialized (the exact replay record byte layout is a G08
  unknown); byte-exact trajectory fidelity is gated on the G02/G03 force
  production. See evidence `impl.sim-vertical-path`.
- `cmr-core::weather` reconstructs the weather / particle / shadow / fog
  environmental behavior (E10). Weather is not a top-level enum at the particle
  layer: the stage's country/surface index (config byte `+0x168 & 0xf`, 0-8,
  forced 7 for the special stage) and the surface-material ids present on the
  stage drive per-effect capability tables (`FUN_00424a40`) that decide which
  effects spawn - reconstructed as `accumulate_effect_enable`, which sums a
  caller-supplied capability table over the distinct present surface ids (the
  tables themselves are game data and not vendored). A scalar weather-type
  (`3`/`4` = rain/wet, matching the H03 thunder/wet audio) and an intensity
  drive the sky/fog and the full-screen precipitation overlay; the detail-level
  `8` disables shadows and floors particle alpha. The `particles` submodule
  carries the pool/record layout constants, the `particle_effect` type dispatch,
  the spawn-frame average (or `0x32` low-detail floor), the `3.0 < dist < 100.0`
  cull window, `follow_size` growth+clamp, and `particle_alpha`
  saturate+low-detail floor; `tuning` holds the named text-overridable defaults
  (`Track_Decay_Life`/`RATE`, dust sizes, per-class alpha deltas). The `shadow`
  submodule has the grey `(100,100,100,170)` blob color, the soft-edge per-vertex
  alpha ramp, the ground-plane flatten, `fixed_to_float`, and the
  `ShadowPrimitive` geometry (4-vertex strip / 11- / 15-index). The `sky`
  submodule has the complete `fog_settings` weather/intensity/country selector
  and the `.hzn` horizon-skip gate. `MAX_DUST_SIZE` is the native 2.0 Q16.16
  clamp. Thirty-four tests cover particles, render queues, shadows, fog and
  tuning. Surface and sprite tables remain caller-supplied data; E10 is complete.
  See evidence `impl.weather-effects`.
- C06's exact settings/profile formats are complete with record/progression
  semantics and round-trip coverage; unassigned fields stay opaque and preserved.
- C05's portable settings/path backend is complete: explicit roots, exact-size
  settings, record tables and profile paths work without registry dependencies.
- B12's lifecycle is complete in both SDL loops: focus loss pauses, focus gain
  restores surfaces, close shuts down, and fullscreen uses the same live windows.
- B11's native presentation is complete: physical pixel-size queries and F11
  fullscreen toggling work in every SDL loop, verified by an offscreen SDL test.
- B10's native keyboard/gamepad acquisition is complete through the recovered
  mapper, analog dead-zones and hotplug handling; DirectInput is enumeration-only.
- B09's native host clock and event loop are complete: `WallClock` drives the
  recovered frame driver, 25 Hz catch-up, render skipping and windowed lifecycle.
- B01's executable map covers 2,168 procedures, 32,530 symbols and 2,310
  defined strings across six memory blocks. Indirect calls and uncarved code
  remain explicit unknowns rather than a false completeness claim.
- B02's launcher lifecycle is recovered: video teardown, configured `Rally.exe`
  path construction, `CreateProcessA`, failure reporting and quit. Successful
  process execution remains outside static analysis.
- E02's entry semantics are complete: vertex/face fields and RGBA order are
  consumer-proven, `field_22` is not a rasterizer input, and E03 resolves binding.
- E03's material variants are complete: indexed truecolor rows preserve
  four-pixel alignment and animation composites resolve selected frames.
  Direct 24-bit is explicitly unsupported in the indexed sampler, not quantized.
- E04's object transform/animation state is complete: `section_neighbors`
  computes Z/X/diagonal links, `relocate_section` preserves list order while
  moving objects, and `mark_world_dirty` propagates through ancestors. Nineteen
  tests cover hierarchy, spatial transitions and dirty state.
- E06's primitive batching, ordering, depth, transparency and blending contract
  is complete alongside E07's concrete renderer. Capability level, alpha gate
  and force-draw override are resolved.
- E07's deterministic renderer/oracle is complete through transform, clipping,
  culling, Z-buffer occlusion and real stage composition. Shipped-binary
  differential capture is explicitly unsupported under the static policy.
- The imported-stage benchmark supplies representative render comparisons with
  a documented 0.1% sampling-tie budget; both opt-in GPU conformance tests pass
  alongside the 9 source-owned image tests. E11 is complete.
- `cmr-test-support::image_compare` is the differential-verification primitive
  (E11) that closes the recovered format work. The reconstruction order ends
  each subsystem by verifying behavior against a differential fixture; the
  software raster emits canonical 8-bit RGB and, after the framebuffer
  conversion, packed 16-bit words, so a faithful check cannot demand
  bit-exactness through 5-6-5/5-5-5 quantization or the documented
  bilinear/dither noise. The module supplies an `Rgb888Image` holder, a
  `Tolerance` model (per-channel delta + accepted outlier fraction, with `EXACT`,
  `RGB565_ROUNDTRIP`, and `dithered` presets), `compare_rgb888` (reporting max
  channel delta, outlier count, first-outlier coordinate, and the verdict), the
  `expand_channel`/`rgb565_to_canonical` bit-replication used to diff a packed
  framebuffer against an 8-bit oracle, and source-owned synthetic fixtures. It
  is deliberately independent of `cmr-video` so the compatibility software
  raster and the later modern GPU backend are checked against the same metric;
  concrete per-subsystem reference captures and their accepted tolerances are
  produced as each render path is converted. See evidence
  `impl.image-comparison-harness`.
- `cmr-core::present_scaling` is the backend-neutral presentation-scaling
  contract (B11), the start of the modern-backend phase. The recovered
  DirectDraw frame-completion ops (`cmr-core::presentation`) flip a back buffer
  that is already the game's logical resolution, so the original did no window
  scaling; a modern backend owns a resizable window whose drawable size rarely
  matches that resolution and must decide how the logical frame is placed.
  `fit_viewport` defines that placement once - aspect-preserving integer or
  fractional scaling (or stretch), centered with letterbox/pillarbox bars, a 1x
  clamp with centered overflow for an undersized surface - so the SDL backend
  and a later GPU backend agree pixel-for-pixel. The `cmr-platform-sdl3`
  presenter selects its SDL logical-presentation mode through
  `logical_presentation(DEFAULT_SCALING)` (`IntegerScale`), implementing the
  project interface rather than a hardcoded constant; the GPU backend will set
  its viewport from `fit_viewport`. Window/renderer creation, the event queue,
  gamepad hotplug, and frame upload already live in `cmr-platform-sdl3`
  (`present_software_frame`, `run_interactive_root_frame`); focus/pause/restore/
  shutdown wiring is B12. See evidence `impl.present-scaling`.
- `cmr-core::host_lifecycle::HostLifecycle` is the end-to-end native host
  lifecycle (B12), composing the recovered host loop (`host_loop`, B09) and the
  recovered window procedure (`window`) into the single lifecycle a modern
  backend drives. A backend-neutral `PlatformLifecycleEvent`
  (`FocusGained`/`FocusLost`/`CloseRequested`) is applied to both recovered
  state machines at once: a focus gain resumes the loop and emits the recovered
  `RestoreLostSurfaces` effect, a focus loss parks the loop and latches an owed
  surface rebuild, and a close request sets the loop quit flag - the recovered
  shutdown path, since the window procedure swallows `WM_CLOSE`. The returned
  `LifecycleTransition` tells the backend what to do (effects, paused,
  shutting_down, surface_rebuild_owed); `next_action` runs frames or parks,
  `take_surface_rebuild` consumes the restore latch, and quit wins over an
  inactive window. The `cmr-platform-sdl3` adapter surfaces the events from SDL
  (`HostEvent::FocusGained`/`FocusLost` from `WindowEvent::FocusGained`/
  `FocusLost`/`Minimized`, `HostEvent::as_lifecycle` bridging quit/focus to the
  contract). This completes the native host window/focus/presentation exit gate
  (B09-B12); the live game loop consumes `HostLifecycle` once the full loop is
  SDL-driven. See evidence `impl.host-lifecycle`.
- `cmr-core::settings_store` is the portable settings/path backend (C05). The
  shipped game loads a single fixed `0x1e20`-byte options blob from
  `save\cmRally.cfg` under a registry-configured writable base, validates it by
  an exact-length gate alone (no header/magic/version/checksum), zeroes a
  runtime region after load, and gates saving on a do-not-save flag; save games
  are twelve `0x6ac`-byte `SAVE\SAVESLOT<n>.cms` slots, a wrong-sized slot
  treated as empty. The original resolves its HD/CD base paths from
  `HKLM\SOFTWARE\Codemasters\Rally\1.00` with absolute fallbacks and fixed
  `new_fe`/`game` subdirs. Per AGENTS.md that registry + absolute-path strategy
  is deliberately not reproduced: `DataPaths` takes an explicit writable
  user-data root and an explicit `--source`/`CMR_ORIGINAL_DIR` media root and
  builds every path with `std::path`, preserving the recovered layout, while
  `SettingsImage` reproduces the exact-length gate, the do-not-save flag, and
  the runtime-zero region, and `save_slot_is_occupied` the slot size gate. The
  shipped `.data` default blob is game data and is not vendored (a caller
  supplies its own default image). This unblocks native startup (C13), the
  interactive native menu (D18), and the profile/persistence flows (F03/F07).
  See evidence `rea.official-eu.settings-paths` and `impl.settings-store`.
- `cmr-app` threads the portable settings/path backend through native startup
  (C13). The executable parses a command and a writable runtime-data root and
  runs the root frame headless or windowed; C13 adds `--source <media root>`
  with the `CMR_ORIGINAL_DIR` environment fallback - the portable replacement
  for the shipped registry `CDPath`/`HDPath` lookup - resolved optionally so the
  headless demo still runs without media while an empty value is a typed error.
  `RuntimeConfig::data_paths()` pairs the two roots into a
  `settings_store::DataPaths`, and `resolve_startup_settings` reproduces the
  recovered load gate (absent -> defaults, exactly `0x1e20` -> loaded with the
  runtime region cleared, wrong length -> rejected). This is reconstruction
  policy, not a decompiled sequence: the registry/absolute-path startup is
  replaced by explicit roots, the settings resolution is pure over injected
  bytes, and the live game loop consumes the resolved paths with the B12
  `HostLifecycle` once the full loop is SDL-driven. See evidence
  `impl.native-startup-config`.
- `cmr-platform-sdl3::advance_root_lifecycle` wires the B12 host lifecycle into
  the interactive native root menu (D18). The presenter already drives the
  recovered `RootInteraction` controller in a resizable SDL window; this reducer
  folds focus/close host events through `cmr-core::host_lifecycle` into a
  `RootLoopDecision { quit, active, restore_surfaces }`, and the loop now pauses
  ticking and rendering while the window is unfocused (blocking on the next
  event), re-presents the frame when a focus gain restored surfaces, and quits
  on a close request alongside the controller's Escape/exit. The reducer is pure
  and tested without a display server; the SDL window/event loop around it is
  glue. See evidence `impl.interactive-native-root-lifecycle`.
- `cmr-core::profile_store` is the save-slot/profile record model (F03). A save
  game is one `0x6ac`-byte record written whole to `SAVE\SAVESLOT<n>.cms` (12
  slots): a header - seed cookie, magic `0x12348765`, format marker `0x690` -
  wrapping a `0x690`-byte championship-state payload with an always-zeroed
  volatile window, a 2-bit mode byte, and a `~seed` complement check. A slot is
  valid/occupied when the magic, format, and complement match, and the loader
  additionally requires an exact-length file; the triple is an anti-tamper gate
  over the seed, not a payload checksum. Creating a profile is the same path as
  saving (a fresh record serialized over any slot). There is no delete
  primitive in the shipped build - a slot becomes empty only by failing the
  validity/size gate - so a portable host deletes by removing the file. The
  module reconstructs the structural header (`make_seed`/`seed_complement`,
  `slot_is_valid`, `classify_slot`, `slot_mode`, `finalize_slot_for_save`)
  while treating the `0x690` payload as opaque game state (offset/length only,
  not vendored). It builds on `settings_store`'s slot length/count and pairs
  with the `DataPaths::save_slot` path builder. `apply_slot_to_live` restores the
  256-byte results region, six-bit progress, difficulty and loaded flag without
  mutating on invalid input; `ProfileSlots` owns the 12-record cache and release.
  F03 is complete. See `rea.official-eu.save-slots` and `impl.profile-store`.
- The persistence round-trip suite
  (`crates/cmr-core/tests/persistence_round_trip.rs`, F07) verifies the settings
  image (C05) and the save-slot record (F03) together: a valid write survives a
  read back unchanged, settings load-then-persist is a stable fixed point, the
  do-not-save flag keeps the on-disk file, and every corruption, truncation, or
  absence recovers to the shipped-defaults / empty-slot behavior rather than
  loading garbage - including the recovered no-delete overwrite. Championship
  records also survive repeated settings round trips. Nine integration tests use
  synthetic, source-owned fixtures. Original-written differential fixtures are
  explicitly unsupported without a supplied original save, and creating one by
  executing the shipped binary is outside the static-analysis policy. See
  evidence `impl.persistence-round-trip`.
- `cmr-core::championship_model` recovers the championship content addressing
  structure (F05) that the `0x690` save payload and the live struct share - not
  the content itself. Dimensions: 9 countries (live `+0x168 & 0xf`, each a `.TIM`
  file via a pointer table), 14 cars (a 4-bit nibble over five parallel 14-entry
  tables, with a numeric stats entry of stride `0x0c`: primary/"BHP",
  secondary/"MM", a 3-way class enum), and 3 difficulties. The save record
  mirrors the live struct by `record_off = live_off - 0x20` /
  `payload_off = live_off - 0x2c` (verified against the car nibble, country
  index, and the result-slot array). The `0x10`-stride result slots contain
  eight position bytes, an entered/order flag at `+0x08`, a tiebreak at `+0x09`,
  and six preserved unassigned bytes; cumulative time immediately precedes each
  slot. `leading_result_slot` selects the first entered order-zero slot. The
  per-slot display descriptor packs `(country&7) | ((stage&7)<<3) | (mode<<6)`
  and maps localized string ids (location base `0x3c`, mode base `0x2f`); the
  Rally-School selector `cfg[0x6c0]` packs task (`>>5`) and difficulty
  (`(>>3)&3`), persists in `cmRally.cfg`, and is outside the championship save
  payload. The mode proxy is live `+0x18 (>>3)&3`. Country/car/stage names, car
  codes, stat figures, difficulty labels and `.TIM` tables remain game data.
  F05 is complete. See `rea.official-eu.championship-model` and
  `impl.championship-model`.
- `cmr-core::championship_progression` recovers the progression/standings/records
  structure over that model (F06). The live struct holds a 16-slot, `0x10`-stride
  results table keyed by car-type slot - cumulative time at `+0x16c + slot*0x10`,
  per-stage finishing positions at `+0x170 + slot*0x10`, and a state flag at
  `+0x178 + slot*0x10` (bit `0x10` = entered the current stage, low nibble =
  provisional order). **Standings are by elapsed time, not points** - there is no
  points table in the game (the module ranks by ascending cumulative time and by
  an accumulated-position score `Σ positions * 10 + tiebreak`, each with a
  rank->slot inverse). Best/record times live in the mutable data image
  `PTR_DAT_0050622c` at `(country*7 + stage)*0x1e + 0xffc` (championship) or
  `stage*0x1e + 0x175e` (alt), stride `0x1e`, 7 stages/country - not in the save
  payload and with no standalone record file. Mode/flags decode over `+0x166`
  (game-mode enum `& 0x1c`, championship-type flag `0x40`, sub-flag `0x20`,
  record sub-mode `& 3 == 2`) and `+0x168` (country `& 0xf`, stage `>> 4`), with
  the `is_championship` and two-player head-to-head predicates. The opponent AI
  time tables (procedurally generated per stage), record times, and stage names
  are game data and are not vendored - only the offsets, strides, ranking
  formulas, and predicates. The ordinary post-stage writer `FUN_00436f00`
  increments the six-bit progress counter and packed stage, preserving country
  and reporting completion without rollback; `FUN_00437090` handles country
  advance/reset and clears all 16 cumulative-time slots. The direct ranking
  accesses resolve the former base ambiguity: time is `+0x16c`, positions are
  `+0x170`. Both record tables are inside the `0x1e20` settings image saved as
  `cmRally.cfg`; checked ranges and a settings round-trip cover that persistence.
  `FUN_0044da00` derives rival selection from country and live standings, with
  the forced-country lookup implemented. Sub-mode 1 and sparse custom sub-mode 2
  scheduling are implemented with fixed/variable limits, forced countries,
  two-player reset, exhaustion and safe invalid-table rejection. F06 is complete;
  availability globals and table contents remain caller/game inputs. See the
  championship progression, stage-commit, record-persistence and custom-schedule
  evidence rows.
- C13's native startup is verified end to end: the real headless session finishes
  and the native window lifecycle is wired. Optional official coefficient files
  remain a separate data-provisioning concern.
- State-4 loading art is a separate app resource, not one of the stage bundle's
  14 siblings. `StageRenderResources` reads the selected stage's user-owned
  Windows `.LOA` in place (raw or gzip/CMPR), validates its 640x480 indexed-BMP
  shape, and presents it only on `LoadInterstitial`. Shared BAR/LINE artwork is
  authenticated through the explicit runtime-data `AssetStore` using paths
  derived from the recovered constants, not from a hard-coded installation
  root. The software compositor draws the progress-scaled BAR first and the
  static 1px LINE from a 1x1 source second, using source index zero as the key
  and the shared recovered clipping plan. The observed normal-stage cap is
  `0x1a` at callsite `0x421fd1`; the synchronous harness presents it for its one
  state-4 frame, transitions to state 5, and does not step simulation or invent
  a progressive delay. Center-nearest sampling and active-palette L1 remapping
  are deterministic software policy. The ordinary RGB555/565 packing formula
  is recovered separately; the DX5 stretched raster and an
  RGB16-to-active-indexed presentation bridge are not. PS1 LOA is TIM-based and
  stays reference-only.
- F01/F04 are complete: `widget_callback` routes value adjustments, accept,
  cancel and screen links, joining the recovered root graph, options, profile
  and championship setup. Menu families share the descriptor engine.
- `cmr-core::frontend_menu` reconstructs the F01 four-phase screen-transition
  spine: interactive descriptor result, transition setup, fade-out and fade-in.
  It emits explicit next-screen, enter, quit and save-and-quit actions. Its
  descriptor walker skips invisible/unselectable widgets, handles wrapping and
  screen links, and edits type-3/6 values with native edge behavior. Six tests
  cover transitions, navigation and values. F02 is complete; F01 retains
  screen-specific graph/callback recovery.
- The F02 frontend-configuration structure spans three modules.
  `cmr-core::options_screen` models the settings-header encoding the options
  screens edit (volume channels `0x0c..0x0f` 7-bit + master flag, display-flags
  byte `0x10` with a 2-bit mode enum, detail bytes `0x11..0x18`, difficulty dword
  `0x1c`, a 3-bit option enum at `0x20` with `4` folded to `0`, and assist flag
  bytes `0x2b..0x30` mapping to the live sim struct `+0x23..0x28`) as bit-exact
  get/set helpers that preserve unknown bits. `cmr-core::control_bindings` models
  the per-player binding record (stride `0x9c`: 8 bindings of key-mask `u16` at
  `+0x64` and physical code `u32` at `+0x78`, device index `+0x46`, device-type
  enum `+0x48` mapping `{0,2,4}` to `{0,1,2}`, control bits `+0x44`), the
  single-bit key-mask to live-slot key-code index mapping (`FUN_0049d3f0`), the
  live input-slot array (stride `0x8c`, 18 slots), and the action bitfield (4
  directions, 12 buttons, any-button `0x10000`). `cmr-core::input_calibration`
  models the fixed-dead-zone analog scaling (`FUN_0049cdf0`: center `0x8000`,
  dead-zones X=2000/Y=2300, rescale and double, half-deflection registering as a
  digital direction) as the compatibility baseline with the dead-zone
  parameterized. The game has **no interactive calibration screen** and **no
  per-action key-rebind UI**: calibration is two fixed `.data` constants and the
  keyboard map is built-in code defaults persisted through the blob->live fixup.
  The default `VK_*` key tables, the device backends (keyboard `GetAsyncKeyState`,
  WinMM `joyGetPosEx`, DirectInput used only for joystick enumeration), the menu
  render engine, and the unlabeled feature meaning of individual flag bits are
  not vendored - only offsets, strides, bit layouts, enum ranges, and the scaling
  formula. See evidence `rea.official-eu.options-screen`/`impl.options-screen`,
  `rea.official-eu.control-bindings`/`impl.control-bindings`, and
  `rea.official-eu.input-calibration`/`impl.input-calibration`.
- The portable compatibility slice (`cmr-core`, `cmr-formats`,
  `cmr-test-support`, `cmr-assets`, `cmr-video`) is verified Windows-portable at
  the source level: it type-checks for `x86_64-pc-windows-gnu`
  (`cargo check --target x86_64-pc-windows-gnu -p cmr-core -p cmr-formats
  -p cmr-test-support -p cmr-assets -p cmr-video --all-targets`) with no
  Linux-only assumptions - production code uses cross-platform
  `std::fs::symlink_metadata` and the only platform-gated code is `#[cfg(unix)]`
  test helpers Windows skips. The path-safety rejection in
  `FileWaveSource::safe_path` and `cmr-assets::resolver` handles Windows path
  shapes (backslash traversal, UNC, drive-qualified), covered by tests. The SDL
  host crates (`cmr-platform-sdl3`, `cmr-app`) require native SDL3 provisioning to
  build for Windows (gated with the Linux displayed run under I07). See evidence
  `impl.windows-portability`.
- CI (`.github/workflows/ci.yml`, I05) runs three jobs on push/PR to `main`: a
  portable gate (fmt + clippy `-D warnings` + test over the SDL-free slice), a
  Windows cross-check (the I04 `cargo check --target x86_64-pc-windows-gnu`), and
  a full-workspace clippy+test on an ubuntu/windows matrix that builds SDL3 from
  source via the `cmr-platform-sdl3` `bundled-sdl3` feature (a passthrough to
  `sdl3/build-from-source`, so no system SDL3 is needed). The first two jobs'
  commands are locally verified; the SDL source-build jobs are authored but not
  yet run under Actions. See evidence `impl.ci-workflow`.
- The native GPU comparison now has an API-neutral presentation slice.
  `cmr-gpu-contract` validates borrowed indexed frames, defines canonical
  opaque RGBA8 expansion and readback normalization, reports capabilities and
  aligned wall-clock timing intervals, and owns the shared synchronous backend
  trait. `cmr-video-wgpu` implements the slice with integer `R8Uint` indices and
  a 256-entry `Rgba8Uint` palette. `cmr-video-sdl-gpu` implements the same slice
  through SDL_GPU's Vulkan backend, keeping its narrow raw-SDL bridge private.
  `cmr-gpu-conformance` validates its all-index and palette-mutation fixture
  against `cmr-video::SoftwarePresentedFrame`, then requires exact readback
  equality from each candidate and across candidates. This was the Phase 4A
  presentation primitive; the later raw-C3D benchmark selected wgpu, while
  device recovery remains open. See evidence
  `impl.gpu-indexed-palette-conformance`.
- The Phase 4B native-geometry slice is `cmr-gpu-contract::C3dRawFrame` and
  `RawC3dGpuBackend`. Its stage-upload vertex retains XYZ and normal as six
  Q16.16 words and carries the recovered prepared UV pair as exact `f32` bits,
  alongside compact triangle indices, resource ordinals, draw brackets, and
  exact matrix words. `cmr-gpu-conformance::PreparedRawStage` builds that stream from
  `StageScene` and `StageC3dTextures` without CPU transform/clip/cull work.
  Both GPU candidates execute the bounded opaque-COPY profile with point/clamp
  indexed materials, exact L1/lowest-index active-palette mapping, perspective
  UV, legacy CCW culling, and writable `LessEqual` depth. Their readbacks match
  exactly on the shared parsed synthetic C3D fixture. Persistent scene ownership
  and representative imported-stage timing subsequently closed the I08 gate and
  selected wgpu; this paragraph records the bounded Phase 4B contract. See
  evidence `impl.gpu-raw-c3d-conformance`.
- The selected wgpu backend now supports optional source-index color keys with
  an `Rg8Uint` material texture: resolved palette slot plus an independent key
  mask. A keyed fragment is discarded before color and depth writes, and an
  explicitly run Vulkan test proves the resulting depth hole. The software
  multi-graph oracle now follows the same `LessEqual` rule, including later
  submission winning equal-depth ties. The single main-stage adapter remains
  explicitly unkeyed; the following layered-foreground and horizon milestones
  close the then-open combined path. See evidence `impl.gpu-c3d-color-key`.
- Main/OBJ/SHT/car foregrounds prepare as one persistent wgpu
  scene in exact layer order. Resource ordinals remain local to each source C3D
  during resolution and are offset into one upload; only car node WORLD values
  and the camera VIEW change per frame. Exact propagated car node worlds retain
  the WORLD_ROOT_CLASS exception. A Vulkan app test matches the software keyed
  layered oracle at two cameras. See `impl.gpu-layered-foreground`.
- The selected backend now executes that HZN boundary as two ordered GPU
  passes into one indexed attachment: horizon clears color/depth with
  `HORIZON_Z_FAR`; foreground loads color and clears depth; one readback feeds
  the existing host. Camera+ARS recompute exact horizon node worlds each frame.
  Synthetic full-layer equality is byte-exact, and authenticated AU1 with the
  retained car passes the established 0.1% raster boundary. See
  `impl.gpu-horizon-pass`.
- The full SDL game-session callback now receives edge-triggered
  `FrontendInput` from the same keyboard/gamepad events as held driving input.
  Focus pause leaves pending edges untouched. This is the host prerequisite for
  replacing the fixed root render with `RootInteraction`; see
  `impl.windowed-frontend-input`.
- Four-wheel stepping adds live forward force along the current heading before
  summing lane forces, so throttle produces world displacement. The native
  session boots directly into the stage rather than auto-advancing the menu.
  This is explicit playable policy, not a recovered original force model.
- `GamepadDrivingState::has_driving_input` gates analog selection: idle pads
  fall back to keyboard, while non-neutral axes/buttons use the recovered analog
  driver. This fixes connected-gamepad keyboard masking without altering either
  mapper.
- Native stage setup uses authored `.cat` nodes, real country palettes, layered
  OBJ/SHT/HZN scenery, ARS horizon settings, and the `.csp` player-one pose for
  both car and first camera. The model adapter now maps local `+X` front and
  local `-Z` right into the harness with a positive determinant, and the
  recovered input half-channels change sign only at the explicit planar
  RaceState control boundary so left/right mean geometric world sides. A
  controlled native wgpu run confirms geometric right steering, progression to
  43 MPH, and no renderer fallback. The latest user capture withdraws that
  run's earlier rear-facing and stable-pacing interpretations.
  CAT-derived startup is fallback only. Follow-up static recovery shows that
  `FUN_00480890` passes the CSP position into `FUN_00404440`, which constructs
  native-basis corners, solves authenticated HPC lanes and applies
  `center_y += lane0_road_y - lane0_corner_y`. The app authenticates the
  extracted selector map and compiled case, owns fresh-process zero-filled lane
  state, and applies only this setup-time Y before the first camera/render.
  Owned AU1 raises Y by `40857` Q16 (~0.6234); forced wgpu then shows the full
  body above the road. The live executable still uses planar
  `GameplaySession::new_with_car`; the SDL host now gates it to one 25 Hz tick
  per 40 ms monotonic deadline, but absent collision/contact and static
  car/wheel animation remain active native defects, and the
  recovered four-wheel producer is not yet the live path. A controlled A/B crop
  confirms the retained adapter shows the rear; the inverse shows the front.
- The windowed planar state-5 path now runs the retained parsed static scene
  after integration. It publishes depenetration, alternate velocity, next-tick
  corrections, history and persistent flags atomically; collision failure rolls
  back before FSM/progress/tick commit. Synthetic tests cover contact publication
  and rollback. The long authenticated AU1 run reached its final steering
  assertion without collision failure and exposed an inverted harness sign;
  recovered right-minus-left is now preserved and pinned for keyboard/analog.
  Concrete damage, angular consume/clear, per-tick contact count, four-wheel
  basis evolution and car/wheel animation remain open.
- `FUN_0040a4a0` decays the prior moment `+0x3d8..+0x3e0` in place by `+0x48`
  and uses the decayed value; its caller `FUN_00407690` then clears it. A moment-overflow trigger (wrapping `|x| + |z|` of the
  decayed moment strictly above `0xbb80000`, only while `+0x680 == 0`) sets
  `+0x680/+0x688` and snapshots the projection axis in the basis frame into
  `+0xbc..+0xc4`; out-of-control ticks convert angles without the half-delta
  term. The momentum order is `qmul(-qmul(combined, inertia), dt)`. These are
  reconstructed and published by the four-wheel step. The angle deadzones and
  resting flag `+0x700` are observed but not yet reconstructed, and the live
  planar session does not yet run this pass.
- The low-speed position suppression reads the new `+0xec`, recomputed from the
  new velocity before the test, and the four-wheel step gates it on `+0x742 > 0`;
  both inputs are corrected. The angle deadzones and `+0x700` are now pure tested
  gates, unwired until the raw steer magnitudes and the control word `+0x404`
  reach the simulation step. The in-race mapper `FUN_00449580` also triggers the
  camera view cycle on a rising edge of device bit `DAT_0055aeb0 >> 16`.
- The per-car mechanics driver `FUN_00407690` clears `+0x3cc..+0x3e0` after the
  integrator every tick, so `+0x3d8` accumulates from zero in each collision pass.
  An end-of-tick clear now runs in every mechanics-tick method; this fixes the
  four-wheel decayed write-back and the live planar session's compounding
  angular impulses. The driver also writes `+0x404` (an upright-cone test on the
  body-frame projection axis, now reconstructed) and cuts steering below
  `|+0xec| < 0x147`.
- The four-wheel step now runs the full contact-gated tail: it cuts the raw
  steer magnitudes (carried in `Controls` from the input map), publishes
  `+0x404` and `+0x700`, and applies the angle deadzones between angle
  conversion and the control-axis resets. The planar windowed path does not run
  this pass, and the steering consumer `FUN_0041b5b0` is not reconstructed.
- The four-wheel step translates as the integrator does (damping by the previous
  `+0xec`, inverse mass `+0x4c`, pending correction, recomputed `+0xec`), with
  no harness steering, drag, throttle push or lateral grip.
- The control processor `FUN_0041b5b0` ramps the raw pedals into `+0x2d8`
  (throttle response), `+0x2ec` (brake response) and `+0x2f4` (handbrake
  torque) with a quarter-sine ease; the four-wheel step now feeds those and the
  raw pedal words to the drivetrain. Its gearbox and steering parts are not yet
  reconstructed.
- The car constants those passes read (`+0x48/+0x4c`, `+0x50..+0x58`,
  `+0x148..+0x184`, `+0x408`) are derived from a car's compiled case words by
  `derive_compiled_four_wheel_constants`.
- The processor's steering half turns the cut steer magnitudes into the wheel
  angle `+0x3e0`, which the four-wheel step adds to the lane-source rotation.
  Its rate scalars `+0x44c..+0x45c` and the steering tables are still inputs.
- The processor's gearbox (automatic, manual, brake-to-reverse) owns the gear,
  the deferred shift request `+0x40c` and the reversing mode `+0x408`; an
  optional live gearbox in the four-wheel step feeds them to the drivetrain.
- The per-player car-setup block (`DAT_0050d950 + slot * 0xcd0`) supplies engine
  response, steering factor, wheel constants, base angle and the gear-2 skip;
  the engine force is split into the four drive slots before the wheel passes.
- The wheel-force producer's brake torques replace the harness drag, and its
  grounded/airborne and per-lane contact gates are in place; the second loop's
  tyre inputs are still harness values.
- The ground pass `FUN_00410a60` keeps the four-wheel car on the stage surface
  every tick (requery, normal, basis alignment, penetration lift, velocity
  constraint); contact flags and the threshold scan are now live.
- The integrator head runs the grip assist `FUN_0040b240` and the rollover
  trigger `FUN_0040bfe0` before the native contact sum, which adds each
  touching probe's support and lane force and its moment about the probe
  offset; an out-of-control car takes the tumbling pass `FUN_004115d0`.
- A wrecked, sunk or stuck car (latch `+0x67c`) is reset by `FUN_00467210` and
  `FUN_00414850`: fade out, placement on the last eligible `.cat` node, fade in.
- The `.dat` trigger walk `FUN_00403a00` runs per tick over the car's cell:
  barrier walls (`FUN_00401fd0`) push and bounce the car, kill volumes set the
  reset latch, cursor records move the checkpoint cursor, co-driver calls fire.
- Co-driver calls are spoken by the recovered sequencer `FUN_00444450` from
  the 40-wave bank on the speech channel.
- Verification covers source-owned fixtures, reviewed-import and offscreen SDL
  runs on Linux, and a clean Windows-target `cargo check`. CI defines portable,
  cross-target and full-workspace jobs; native Windows and Actions provisioning
  are unrun. Comparison/deviation matrices and the README now reflect the full
  117-complete / 2-partial roadmap.
- Edition support is limited to the reviewed official-EU/installed-EU profile.
  Other editions and locales are unsupported until separately evidenced;
  official-US and xdelta files are comparison-only artifacts.
- Stage/championship coverage includes nine stage siblings, layered software
  presentation, authored starts, transform/visibility/mutation and retained
  setup sessions. The native root now launches the configured race only through
  its recovered Rally selection. MOJ/SPL/ANM/RSF/FUZ, DAT dispatch, remaining
  zero-copy GPU presentation, finish, and post-race behavior remain open. The player car
  and normal software race HUD are live.
- Bitmap conversion has an explicit deterministic indexed boundary: 4/8-bit
  BM, palettes, stored rows, composition, and queued fonts are supported;
  native padding, 24-bit, driver filtering, device-owned formats, and alternate
  font pixels are explicitly unsupported.
- Type-1 C3D support covers decoded XYZ/normal/UV resources and branch-zero
  flat/COPY indexed rendering. General lighting and alternate Gouraud/modulate
  output are explicitly unsupported rather than borrowed from type 2.
- Raw stage C3D UVs now cross one common preparation contract before software,
  wgpu, or SDL_GPU sampling: strict one-time U subtraction above one and a
  single-rounded V-plus-one bias. Parser/raw carriers remain unchanged;
  frontend-prepared, reflection-generated, and 2D coordinates do not apply it
  twice. Clamp remains a later sampler state. The coherent AU1 capture confirms
  the former row-zero collapse is gone without claiming driver-exact pixels.
- Renderer choice is observable: native startup reports requested/selected
  renderer and any typed setup/frame fallback without paths, and an `auto`
  frame failure warns before permanently using software. The slow validation
  capture explicitly forced software; full-layer AU1 measured about 0.32 s per
  release frame before car/HUD (~3 FPS), while `auto` successfully selects wgpu
  on the current host. Software remains the oracle, not the performance path.
- Final presentation uses owned indexed frames and checked RGBA8 expansion;
  render routes select composition and lifecycle controls presentation. The
  native root combines recovered interaction, descriptor menus, and
  pause/restore/quit handling. DirectDraw flip timing and display-server details
  remain backend boundaries.
- Networking covers per-player 15/16/17/5-word record messages with
  ready/active/done transitions and wrapping accumulation. Unknown fields stay
  positional. Runtime application/provider GUIDs are external identity; the
  optional TCP backend remains one-peer.
- Force feedback includes `SamplePeriodUpdate` and `PeriodicGainUpdate` for the
  final native wrappers, with exact DX7 effect size, set flags and two-byte type
  payload. Runtime surface/car tables remain external inputs.
- Music/video are portable contract implementations: PCM selection/loop/fade
  runs through `AudioHost`, while `VideoBackend` covers open, bounds, play,
  pause, skip and close with tests. CD music and command-line intro skip are
  negative findings. Replay trigger, focus suppression, exact clip suffix/order,
  and a real video decoder remain explicitly unsupported.
- `audio_surface` implements collision impact speed bucketing, primary and
  secondary volumes, sub-wave offset, and the `>333` listener-volume curve with
  native integer order. Vendor units and 3D coordinate meanings stay explicit.
- Audio ABI/loading is bounded by the completed 16-import scope and the two
  exact open-wave forms (`8` named PCM, `0x10` managed). `WaveOpenCall` preserves
  those values without inventing QMixer's vendor flag meanings.
- `camera_view::ViewController` unifies recovered chase/cockpit/helmet/cinematic
  placements, trackside and reset behavior, view cycling, and dual-slot
  re-orthonormalized crossfades. The SDL session drives it from the live car.
  Runtime camera data and the uncovered remap/ride-height/helmet-overlay and
  `.cat` director paths remain explicit boundaries.
- Stage timing now includes `FUN_00461e90` delta normalization before
  checkpoint dots, the existing checked settings-record best-time path, and
  18x20-byte per-driver result rows with decrement/hide-at-zero TTLs. The
  spawn-position writer is explicitly unidentified; result text payloads stay
  opaque.
- `cmr-core::collision` now covers the signed out-of-bounds timer and crash
  reset orientation path: positive countdown/timeout sound, negative terminal
  latch, both basis-row modes, snap/blend, normalization and totalled completion.
  Contact geometry comes from the recovered HPC provider. Reset triggering is a
  caller input; position-snap, time penalties, and damage-performance mapping
  are not inferred. Dynamic trackside objects (`cmr-core::moving_objects` and
  `cmr-core::moving_object_collision`) are loaded from `.obj` (`FUN_00482140`),
  bucketed into the cell grid (`FUN_004622b0`), resolved against the car each
  tick (`FUN_0043f790`/`FUN_00440d90`/`FUN_0043faf0`) with crash damage and
  school cone-contact flags (`DAT_00552f68`, `DAT_00558fb0`), simulated over
  the `.hpc` ground (`FUN_00450d40`), and rendered with live node locals in both
  the software and GPU renderers. Car lamps, pod breakage (`FUN_0044a770`),
  electrics flicker (`FUN_0044b000`), additive flare/beam sprites
  (`FUN_0044bbc0`), and the night road light pool (`FUN_004a8750`/`FUN_004a9490`)
  are reconstructed in `cmr-core::car_headlights` and wired into `cmr-video` and
  `cmr-app`. Car-vs-car collision (`FUN_00442740`/`FUN_004438a0`/`FUN_00442860`/
  `FUN_00441f30`) is reconstructed in `cmr-core::car_vs_car_collision` and
  bridged onto `RaceState`.
- Suspension/surface recovery is composed end to end: authenticated HPC data
  feeds the complete eight-lane initializer, and the four-wheel producer shares
  its force/contact results with geometric postprocess, translation, angular
  integration and driven smoothing. `apply_translation_step` covers cached-speed
  damping and the semi-implicit position term. Runtime car/config/surface data
  and native pointer-fault/rebinding behavior remain explicit safe-contract
  boundaries.
- Driven wheel smoothing uses the first-loop `+0x540` force as native `+0x2c8`
  input rather than a placeholder. The complete four-wheel step selects the
  recovered ordinary/driven/neutral branch per lane, derives slip, direction,
  speed-limit and engine-load scalars, and feeds those scalars to engine-speed
  smoothing. External suspension/longitudinal/response values remain explicit
  inputs in `DrivenWheelUpdateParams`.
- The fixed-step session composition is `cmr-core::gameplay_session`: it plugs
  `race_flow_next` into the top-level `GameSession`, steps the race only in
  state 5, updates checkpoint progress, and exposes the recovered render route.
  Both the deterministic headless command and native SDL loop use it; the
  checked four-wheel path remains covered separately. The playable stage uses
  the existing planar dynamics and explicit control strengths so incomplete
  vertical-force integration cannot destabilize live driving. Authored `.cat`
  checkpoints drive progress; spawn, dynamics and post-race inputs remain
  explicit harness boundaries. The explicit caller-selected in-race C3D car is
  depth-composed under the live pose, and the normal race HUD is applied to each
  presented world frame.
- Rectangular C3D surface expansion does not define the contents of added
  padding. `FUN_0049e960` allocates a DirectDraw surface and `FUN_00499320`
  writes source-derived pixels only, so the indexed sampler keeps GR4's padded
  32x64 resource fail-closed instead of assigning zero, edge, or color-key
  pixels. This is an explicit native-driver ownership boundary; deterministic
  emulation of that allocation state is not supported by current evidence.
- The canonical code reference is the official EU updater payload. The
  installed EU-derived executable, official US payload, and unofficial xdelta
  results are tracked comparison variants.

The fixed-base/no-relocation assumption is part of historical compatibility,
not an implementation requirement for a modern portable binary.

