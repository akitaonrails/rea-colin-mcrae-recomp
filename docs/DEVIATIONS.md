# Supported editions and known deviations

This document states which editions of the original the reconstruction targets
(roadmap item `I13`) and every place where the reconstruction intentionally or
necessarily differs from the original. These deviations are not defects to be
closed; they are declared differences. Behavior still being recovered is tracked
in [DECOMP_MAP.md](DECOMP_MAP.md), not here. How any of this is verified against
the original is in [COMPARISON.md](COMPARISON.md).

## Supported-edition matrix

The reconstruction is anchored to one canonical artifact; the others are tracked
only as comparison variants. Identities are recorded in full in
[EVIDENCE.md](EVIDENCE.md); hashes here are display-only abbreviations.

| Edition | Artifact | Role | Status |
| --- | --- | --- | --- |
| Official EU updater payload | `Rally.exe`, PE32 i386, base `0x400000`, SHA-256 `0eb6ea96…` | **Canonical reference** - all recovery claims anchor here | Supported |
| Installed EU-derived executable | the on-disk EU install | Comparison variant | Tracked |
| Official US updater payload | US `Rally.exe` | Comparison variant | Tracked, not the recovery baseline |
| Unofficial xdelta result | community-patched binary | Comparison variant | Tracked, not endorsed |

Edition policy: a developer supplies their own legally-owned media through an
explicit `--source` root (or the documented `CMR_ORIGINAL_DIR` example
variable). The project ships no executable, DLL, media, or data. Editions other
than the canonical EU payload may differ in offsets and are not guaranteed to
match recovered addresses.

## Known-deviation matrix

Each row is a difference the reconstruction makes on purpose, with the original
mechanism, the replacement, and the reason. "Item" cross-references the roadmap.

| Area | Original mechanism | Reconstruction | Reason | Item |
| --- | --- | --- | --- | --- |
| Media/path resolution | Windows registry `HKLM\…\Rally\1.00\{CDPath,HDPath,Hardware}` + absolute fallbacks + fixed `\new_fe`/`\game` subdirs | Explicit `--source` / `CMR_ORIGINAL_DIR` media root + a writable user-data root, paths built portably with `std::path` | No registry, no committed absolute dev paths; portable and OS-neutral | C05, C13 |
| Save/config location | config and `SAVE\SAVESLOT<n>.cms` under the install/registry base | Same record formats under the caller-supplied writable runtime-data root | Portable, user-writable, no install-dir assumption | C05, F03 |
| Presentation / 3D | **Direct3D 5 Immediate Mode** on `IDirect3DDevice2` over DirectDraw, hardware 16-bit Z-buffer, `DrawIndexedPrimitive`/`D3DLVERTEX`, per-object painter sort (no triangle sort); confirmed by E06 | SDL3 integer-scaled nearest streaming presentation now; the selected portable GPU backend is **wgpu** (I08 measured it 1.7-2.5x faster than SDL_GPU on the draw-heavy recovered stages and it needs no external shader compiler) and it renders the stage/world frames through the persistent raw-C3D contract, reading back the active-palette index plane so the recovered indexed presentation is unchanged, with the software compositor as oracle and fallback (`--renderer auto|software|gpu`) | Remove the DirectX requirement; keep the logical-resolution baseline and the recovered blend/depth semantics | E06, I07, I08, I09 |
| Audio mixing | QSound `QSWaveMix*` (QMixer), 22050 Hz / 30 channels | Deterministic reconstruction-owned software mixer + SDL3 audio device, same recovered init/slot/channel model | Remove the QMixer DLL requirement; make mixing testable and deterministic | H04, I07 |
| Music | Looping PCM `frontend.wav` / `replay.wav` via QSound; **no CD/Redbook/MCI path exists in the EU build** | Same PCM looping path driven as mixer commands; no CD subsystem added | Faithful: the absence of CD music is a recovered fact, not a removed feature | H05, H07 |
| Full-motion video | Windows MCI `avivideo` (`.avi` via `mciSendCommandA`) | Portable `VideoBackend` interface mirroring the recovered MCI calls; concrete decoder is backend glue | Remove the Video-for-Windows/MCI requirement | H06, H07 |
| In-race input | WinMM `joyGetPosEx` + `GetAsyncKeyState` | Portable edge-triggered keyboard/gamepad mapper driving the recovered input contract behind SDL3 | Remove the WinMM input requirement; one mapper for all backends | F02, I07 |
| Analog calibration | Two fixed `.data` dead-zone constants (no screen, no persisted state) | Same fixed-dead-zone scaling as the baseline, with the dead-zone parameterized behind the input backend | Faithful: there is no calibration UI to reproduce | F02 |
| Key rebinding | No per-action rebind UI located; built-in code default key tables | Default control scheme modeled structurally; a rebind UI is not fabricated | Faithful to the recovered design | F02 |
| Force feedback | DirectInput effects (`DINPUT.dll`) | Recovered effect semantics (`cmr-core::force_feedback`); a portable FFB output backend is future work | Remove the DirectInput requirement without inventing effects | H08 |
| Multiplayer | DirectPlay `DPLAYX.dll` host/join sessions, player enumeration, and `Send`/`Receive` messages | Not required by the single-player portable build; an optional direct-connect TCP transport (`cmr-core::net_tcp`) implements the recovered session lifecycle behind the shared `NetworkTransport` contract and its tests | Removes the DirectPlay requirement; recovered player records are implemented, but full multi-peer synchronization is not claimed | H09, H10 |
| Required DLLs | DirectX, QMixer, WinMM, DirectInput | None required by the portable build; SDL3/wgpu/software backends provide the equivalents, and `scripts/check-legacy-backends.sh` gates that no project code, dependency, or built artifact binds the legacy surface | A self-contained portable build | I10 |
| Packaging | Shipped with copyrighted payloads | Binaries packaged without any game payload; media supplied by the user | Legal and redistributable | I11 |
| Launcher / CLI | No in-binary command-line parsing; a separate setup/launcher binary (not in this artifact) | A native `cmr-app` with explicit `--source` / `--runtime-data` options | Recovered negative result (no `-skip`/`nointro` tokens exist); portable entry point | H06, I02 |
| Translucent blends on the indexed frame | The 16-bit display blends sprites, tyre marks and fades in colour | The 8-bit indexed frame re-quantizes each blended pixel to the active palette with a 4x4 ordered dither (`cmr-video::software_stage_scene::dithered_blend`) | The indexed presentation has no colour depth to hold the blend; dithering keeps faint dust visible instead of snapping to the underlying colour | I07 |
| Rally school speech channels | Intro wav on its own channel (`DAT_005fc038`), advice and exit wavs on the advice channel (`DAT_005fbef8`), both at volume 32000 | All three play on one host speech channel at the same volume; the busy test still covers every speech channel | The two channels never overlap in practice (each speech waits for the other to finish); one host channel keeps the mixer allocation simple | H04 |
| Handling after a static-scenery impact | `FUN_00420740` re-derives the handling inside the collision pass | Re-derived right after the pass, before the trigger walk and ground pass of the same tick | The words it writes are read only by the force pass, which ran earlier in the tick, so the result is the same | G04 |
| Image base / relocation | Fixed base `0x400000`, no relocation | The compatibility baseline retains the fixed-base/no-relocation assumption | Faithful historical compatibility | B-series |

## Classification

- **Portability deviations** (registry/paths, DLLs, packaging, launcher): required
  to run off Windows-specific services; the recovered behavior is preserved, only
  the plumbing changes.
- **Backend deviations** (presentation, audio, video, input, FFB): a modern
  backend implements an already-defined project interface and is held to the same
  contract tests as the compatibility implementation, so behavior is not
  modernized while it is still being recovered.
- **Faithful non-deviations** (no CD music, no calibration UI, no rebind UI, no
  CLI parsing): recorded here to pre-empt "missing feature" reports - these were
  decisive negative results, not omissions.

## Explicit unsupported boundaries

The portable GPU backend is selected and integrated (`wgpu`). Force-feedback
semantics are recovered, but a concrete portable FFB output device remains
unsupported. Networking covers recovered player records and the session
lifecycle, but full multi-peer synchronization is not claimed by the one-peer
TCP backend. Other editions/locales are unsupported until separately evidenced.

Native stage startup uses node-zero pose and direction as a documented playable
approximation; the shipped spawn writer remains unrecovered. The playable loop
uses the existing planar dynamics and harness control strengths rather than
claiming complete four-wheel trajectory or terrain-contact fidelity. Actual
`.cat` checkpoints, live controls, real country palettes, and a live follow
camera are used; player-car rendering and the in-race HUD are not yet composed.
Post-race replay/flyby/fade/quit flags remain harness inputs. No multiplayer is
required.

Verification boundaries are environmental, not silent passes: original-written
save comparison requires user-owned files; hosted Windows compilation/tests do
not replace an interactive run on every hardware/driver combination; a real
video decoder and native driver-exact font/filter/alpha output are unsupported.
These limits are recorded in
[DECOMP_MAP.md](DECOMP_MAP.md) and are not presented as compatibility claims.
