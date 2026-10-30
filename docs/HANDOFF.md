# Reconstruction handoff

Updated: 2026-11-02

## Resume point

Work resumes from the `main` commit whose subject is:

```text
feat: reconstruct car-vs-car collision and response
```

Start by confirming the checkout and progress footer:

```sh
git status --short
git log -8 --oneline
tail -n 12 docs/DECOMP_MAP.md
```

This milestone completes the car-vs-car collision and response pipeline
(`FUN_00442740`, `FUN_004438a0`, `FUN_00442860`, `FUN_00441f30` in
`cmr-core::car_vs_car_collision` and `RaceState`). It does **not** establish a
finished or fully accurate game.

## What is now implemented

- `RaceState::resolve_static_collision_after_step` builds the exact authored
  primary cell and live car hull/projection after integration, runs the recovered
  kernel, and publishes depenetrated position, alternate velocity, next-tick
  linear/angular corrections, history, and persistent `+0x680/+0x688` flags.
- `GameplaySession::try_advance_with_static_collision` snapshots the complete
  pre-step race state. Collision failure restores it before FSM, checkpoint,
  or simulation-tick publication.
- The windowed app uses that path whenever its parsed static scene is present.
  It currently supplies `NoStaticDamageOwner`, explicitly leaving the recovered
  mutable damage-object implementation open.
- Pending linear correction `+0x3cc` is consumed once by the next translation
  step and cleared.
- Angular `+0x3d8` lifecycle per tick: the collision pass accumulates from zero;
  the integrator `FUN_0040a4a0` decays it in place by `+0x48` and uses it; the
  per-car mechanics driver `FUN_00407690` then clears `+0x3cc..+0x3e0`
  (`rea.official-eu.mechanics-tick-driver`). `RaceState::finish_mechanics_tick`
  reproduces the clear at the end of every mechanics-tick method. An earlier
  milestone wrongly concluded "never cleared" from the integrator alone; that
  is corrected, along with the two defects it left (the four-wheel decayed
  write-back, and the live planar session compounding angular impulses).
- A recovered moment-overflow trigger now runs in the four-wheel angular pass:
  only while `+0x680 == 0`, wrapping `|x| + |z|` of the decayed moment strictly
  above `0xbb80000` sets `+0x680/+0x688` and snapshots `+0xbc..+0xc4`. That
  tick's angle conversion already uses the out-of-control form (no half-delta).
  The momentum order was also corrected to `qmul(-qmul(combined, inertia), dt)`
  (the old form was one LSB off whenever the time-step product had low bits).
  The live planar windowed path still does not run this pass.
- The low-speed position suppression now tests the speed recomputed from the
  new velocity (the native test runs after `+0xec` is updated), and the
  four-wheel step gates it on `+0x742 > 0`, not the wheel flags. The angle
  deadzones and resting flag `+0x700` are now wired into the four-wheel step:
  `Controls::raw_steer` carries `+0x2fc/+0x300` from the input map, the step
  applies the instruction-confirmed low-speed cut (`|speed_ec| < 0x147`),
  publishes `FourWheelState::upright_404` from the start-of-tick body axis and
  `resting_700` from the translation's recomputed speed, and the deadzones run
  inside `apply_angular_orientation_pipeline` between angle conversion and the
  control-axis resets (`impl.steer-cut-deadzone-wiring`).
- The in-race mapper `FUN_00449580` owns the shipped camera-change input: a
  rising edge on device bit `DAT_0055aeb0 >> 16` (default `0x40`) calls the view
  cycle `FUN_00434210` (`rea.official-eu.camera-cycle-binding`).
- The explicit planar `CarControls -> Controls` bridge now preserves the
  recovered `right - left` command. Keyboard, analog, and SDL adapter tests pin
  right-positive, left-negative, and cancellation.

The collision milestone builds on these immediately preceding commits:

```text
a94b0fd feat: add lazy static damage ownership
014607f feat: consume pending static linear correction
14063c9 feat: retain live collision state across ticks
4e71941 feat: publish live collision contact count
ef78049 feat: derive canonical collision response state
2d52d72 feat: recover static response live inputs
330ff19 feat: publish compiled collision setup state
5fb7c36 feat: recover live primary collision cell
```

## Verification state

Green at handoff (2,397 tests, 36 ignored):

```sh
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Focused collision, session rollback, keyboard/analog, SDL gamepad, app build,
and full-workspace tests pass. Logs from the last run are ignored under:

```text
.work/ground-pass-workspace-test.log
```

The authenticated AU1 test:

```sh
CMR_RUNTIME_DATA="$PWD/runtime-data/installed-eu-derived-assets-v2" \
  cargo test -p cmr-app \
  imported_au1_live_input_moves_the_world_and_advances_checkpoints \
  -- --ignored --nocapture
```

It was first run before the steering correction and failed only on left
steering (`+40` instead of `-40`). **It has since been rerun green** at
`b86736c`, after both the sign correction and the per-tick angular moment clear:
about 200 seconds, all assertions including steering (log
`.work/au1-authenticated.log`). It proves collision execution without error and
the steering sign on owned media; it still does not prove a scene object was
contacted (item 5 below).

## Latest state (read first)

The four-wheel car stays on the owned AU1 surface (`impl.ground-pass`) but does
not drive yet: all lane forces are zero, so it slides downhill under gravity
and the checkpoint cursor never advances (a correction to `834e4c9`). Run
`CMR_RUNTIME_DATA="$PWD/runtime-data/installed-eu-derived-assets-v2" cargo test
-p cmr-app --release probe_compiled_four_wheel_car_on_au1 -- --ignored
--nocapture` (it prints the cursor; `CMR_PROBE_EVERY=1` prints every tick).
The tyre gains and bases now come from `cmr-core::tyre_load`
(`impl.tyre-gains-and-live-force-head`), and with the second loop's inputs and
the contact-vector mirror corrected (`impl.second-loop-inputs-and-contact-mirror`)
the car accelerates along its body axis through the gears on AU1
(`CMR_PROBE_TICKS=1500`). With the spawn rows facing the road
(`impl.spawn-rows-face-the-road`) it drives down the road under throttle and
the cursor reaches 21 before the car leaves the ground and tumbles near tick
1000 (the out-of-control path `FUN_004115d0` is missing). Steering and the
drive split are now sourced and the windowed session runs the four-wheel
producer (`impl.steering-and-drive-split`); `CMR_PROBE_STEER=left|right` with
`CMR_PROBE_STEER_AFTER=<tick>` scripts the probe's steering. Rotations now use
the native row-vector transform (`impl.orientation-row-vector-transform`), so
steering turns the right way. To see the car, run the ignored
`imported_au1_four_wheel_window_frames` test (`CMR_FRAMES_STEER=left|right`,
`CMR_FRAMES_STEER_AFTER`, `CMR_FRAMES_TICKS`, `CMR_FRAMES_EVERY`); it writes PPM
frames under `.work/frames`. The out-of-control path is reconstructed
(`impl.tumbling-ground-pass`), and so is the integrator head
(`impl.integrator-head`), which removed the bank-slide blow-up. Run the probe
with `CMR_RUNTIME_DATA="$PWD/runtime-data/installed-eu-derived-assets-v2"`.
The car reset is reconstructed (`impl.car-reset`); the wheel/suspension
animation routines are presentation-only (`rea.official-eu.car-animation-words`).
The HUD rev, speed and gear come from `FUN_00464130` (`impl.race-hud-prepare`).
The `.dat` trigger walk and barrier walls run each tick
(`impl.trigger-walk-and-barriers`), and the co-driver speaks
(`impl.codriver-speech`). The windowed session runs a player's race and returns
to the menu after the finish (`impl.gameplay-script-and-stage-restart`). The wheels are animated and posed (`impl.wheel-animation`). The engine sound
plays (`impl.engine-sound`): extract its tables once with
`cargo run -q -p cmr-assets -- extract-engine-audio-tables --source <dir with Rally.exe> --profile profiles/official-eu-engine-audio-tables-v1.json --output runtime-data/official-eu-engine-audio-tables`;
`CMR_FRAMES_AUDIO=1` on the ignored frames test writes `engine.wav`. Open
points: QMixer panning of `SetPosition` and `car +0x764`. The tyre profile's
`wheel_terms_777c` read (8 variants x 120 x 6 bytes from `0x4f777c`) overlaps
the engine tables at `0x4f82e0..0x4f8bac`; that is the native's own reach:
`FUN_00486090` stores an explicit variant (1 or 2 from `0x4eac78`) or
`cfg[0x91 + p*0x9c] & 7`, so variants 4..7 alias the engine tables exactly as
in the shipped image.
Escape pauses and the menu is drawn (`impl.pause-menu`, `impl.pause-menu-draw`;
`CMR_FRAMES_PAUSE_AT=<frame>` on the ignored frames test taps Escape).
Surface/skid, landing and barrier impact sounds play
(`impl.surface-and-prang-sound`; tables via `extract-surface-audio-tables`).
The wet-tyre sound plays (`impl.wet-sound`). The crowd ambience is wired
(`impl.crowd-sound`) but no shipped `.obj` has a crowd node (id `0xe`), so it
stays silent, as natively; storm/thunder are unwired. The front end runs outside the FSM and
the event flow (state 1 event choice, the hub's advance/rewind/end of
session) is recovered (`impl.event-flow`). Confirming Championship, Rally or
Time Trial at the root starts that event (harness for the setup screens), and
state 4 loads the event's stage (`impl.stage-reload`), so a championship runs
NZ1 onwards. `--source` must be a stage directory `<root>/Game/<CC>/<n>`.
The record persists as `<runtime-data>/save/cmRally.cfg` (`impl.cfg-file`).
Stage weather (`impl.stage-weather`) needs
`cargo run -q -p cmr-assets -- extract-stage-weather-tables --source <dir with Rally.exe> --profile profiles/official-eu-stage-weather-tables-v1.json --output runtime-data/official-eu-stage-weather-tables`;
the rivals' times (`impl.rival-times`) need `extract-rival-records` the same way
(`profiles/official-eu-rival-records-v1.json`, output `runtime-data/official-eu-rival-records`).
Post-stage results are documented in `.work/post-stage-results-notes.md`,
`.work/results-records-notes.md` and `.work/results-pages-notes.md`; standings
are computed (`impl.standings`) and the results director runs every
single-player page (`impl.results-director`, `impl.results-record-checks`): the
stage, rally and championship tables, the record checks with their notice and
CAC code pages, the menu, the confirmations, GAME OVER, the complete pages, the
save page's dialog, the super-special result and the bonus car, all drawn with
the make logos and flags (`impl.results-draw`, `impl.results-icons-pages`). The
super-special opponent's time needs the opponent car, which is not simulated.
The race hand-off commits the cmRally.cfg record (`impl.config-commit`), and
records written by the results reach it. The window's front end runs the real
screens (`.work/frontend-menu-tree-notes.md`, `.work/frontend-setup-draw-notes.md`):
setups, name entries (with the cheat and CAC codes, `impl.frontend-name-codes`),
stage select, Load, Options and its sub-screens, and the control, analogue and
button setups, with their previews, flags, logos and the turning 3D car
(`impl.frontend-car-preview`); the stage load attaches the car chosen in the
setup (`impl.chosen-car`). Before a stage the country welcome screen (state 2,
`impl.welcome-screen`) and the service screen (state 3, `impl.service-screen-draw`,
`.work/service-screen-draw-notes.md`) run from the user's assets; the damage
write-back `FUN_00420db0` is wired (`impl.car-damage`), and crashes dent the car
(`impl.car-crash-damage`, `impl.crash-damage-wiring`, `.work/car-damage-notes.md`,
`.work/car-damage-contact-notes.md`) except from moving objects and other cars.
The race fades follow the binary (`impl.race-fade`, `.work/race-fade-notes.md`);
the rally school's screens run in state 2 (`impl.school-menu`,
`.work/school-menu-notes.md`) with their Video 1 movies (`impl.msvideo1-movies`);
its tasks run in the race with the scorer `FUN_004725e0` (`impl.school-race`,
`.work/school-race-notes.md`), with the status text (`impl.school-text`) and the instructor's advice (`impl.school-advice`), without the intro/exit replays. `--language english|french|german|spanish|italian`
stands in for the registry's installed language (English by default). Static tables (`impl.static-tables`) need
`cargo run -q -p cmr-assets -- extract-static-tables --source <dir with Rally.exe> --profile profiles/official-eu-static-tables-v1.json --output runtime-data/official-eu-static-tables`. Next: the car-to-car impact site
(the `FUN_004115d0` landing and `FUN_00412860` tumbling impacts are wired), then the championship progression
behind the hub predicates. Scenery impacts play (`impl.static-impact-sound`).
`FUN_0041f630` only feeds impact sound/FFB/shudder. Trees appear to be `.sht` sprites, which the native
excludes from static collision. Done so far: the windowed session on
`GameplaySession::new_with_four_wheel_and_car(...).with_ground_geometry(...)`
from `SessionCompiledPose` (`compiled_car`, `geometry`, seeded `CarState::ground`),
render the car body from `MotionState::basis`, then the out-of-control path
`FUN_004115d0`, suspension/wheel animation `FUN_00410350`, and the remaining
tyre inputs (`DAT_004f6b50/6f10/777c` are now in `SessionTyreTables`).

## Latest state (read first)

`.SSR`/`.RSP` file parsing and pointer relocation (`ReplayObjectHeader`,
`ReplayFileView`, `FUN_0046ddc0`/`FUN_0046de30`), Rally School `.rsp` path
selection (`school_intro_rsp_path_46de70`, `school_exit_rsp_path_46de70`),
`CarState`/`RaceState` Region-A seeding (`seed_from_replay_region_a_46d460`,
`Controls::from_decoded_replay_control`), and `GameplaySession` State 7/9
`intro.rsp`/`exit.rsp` playback and `FUN_0046edd0` pre-pan car restore
(`SchoolReplayTrack`, `set_school_replays`, `skip_school_replay`) are now
reconstructed and wired into `cmr-app::start_school_task`
(`impl.replay-file-and-school-rsp`). Replay snapshot keyframes
(`SnapshotKeyframe`, `SnapshotCarLane`), Region-A seed views
(`ReplayRegionAView`), and RLE input-trace playback (`ReplayPlaybackStepper`)
are also complete (`impl.replay-snapshot-and-region-a`), as are all 14 stage
bundle sibling decoders and `.moj` kinematic object motion (`E08` -> complete,
`118 / 119` items complete; `impl.stage-bundle-all-14-siblings-and-moj`).

## Highest-priority remaining work

The earlier items (four-wheel path, contact flags, the static damage owner,
the body basis, moving objects, night headlights, car-vs-car collision, stage
sibling sections/`.moj` motion, and `.SSR`/`.RSP` replay seeding and Rally School
pan playback) are done; see `docs/CONVERSION_TIMELINE.md` for the history. Open
now, in order:

1. The intro FMV codec (IV50 / Indeo 5) for the front end's opening movie and
   final `I14` closure.

The next `docs/DECOMP_MAP.md` focus stays on the gameplay gaps above.

## Evidence and boundaries

- Static collision recovery is documented in `docs/EVIDENCE.md` under
  `impl.atomic-live-static-collision-session` and the related producer/kernel
  records.
- The steering runtime observation is recorded as
  `obs.au1-steering-sign-regression`.
- Local REA and SuperGhidra output stays under `.work/`; do not duplicate shipped
  payloads or add absolute media paths.
- The runtime app still reads user-owned data through the explicit imported-data
  profile and source-root interfaces. Never commit original game assets.
- Keep observations, inferences, design choices, and unknowns separate. Static
  evidence does not establish runtime behavior.
- Update `docs/EVIDENCE.md`, `docs/ROADMAP.md`, `docs/SUBSYSTEMS.md`,
  `docs/DECOMP_MAP.md`, and `docs/CONVERSION_TIMELINE.md` with each milestone,
  and keep the progress footer as the final lines of `docs/DECOMP_MAP.md`.

## Progress monitor

```sh
watch -n 2 'tail -n 12 docs/DECOMP_MAP.md'
```
