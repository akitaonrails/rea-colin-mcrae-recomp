#!/usr/bin/env python3
"""Deterministic Rally.exe -> Rust function & call-graph coverage map tool.

Usage:
  ./scripts/check-function-map.py --generate   # Rebuild docs/function-map.json from .work/rea/
  ./scripts/check-function-map.py --verify     # Verify docs/function-map.json against crates/ (CI-safe)
  ./scripts/check-function-map.py --report     # Print per-FSM-state & orchestrator gap report
  ./scripts/check-function-map.py --state 8    # Show reachable gaps for a specific FSM state
"""

from __future__ import annotations

import argparse
import bisect
import json
import pathlib
import re
import sys
from collections import defaultdict, deque

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
MAP_PATH = REPO_ROOT / "docs" / "function-map.json"
PROCS_PATH = REPO_ROOT / ".work" / "rea" / "all-procedures.json"
DISASM_PATH = REPO_ROOT / ".work" / "rea" / "rally-text.disasm"

EXPECTED_TEXT_PROCS = 2018
TEXT_START = 0x00401000
TEXT_END = 0x004E5C00

# Canonical FSM state handlers (update A, render B) from cmr_core::game_state::STATE_HANDLERS.
FSM_ROOTS = {
    "state_0_stopped": ["0x004398a0", "0x004234a0"],
    "state_1_boot": ["0x00436be0", "0x00421cc0"],
    "state_2_frontend": ["0x00437160", "0x00421cd0"],
    "state_3_service_preload": ["0x004371c0", "0x00421d80"],
    "state_4_load_interstitial": ["0x00437200", "0x00421da0"],
    "state_5_drive_race": ["0x00437ac0", "0x004231f0"],
    "state_6_drive_alt": ["0x00437690", "0x004231f0"],
    "state_7_school_intro": ["0x00437ef0", "0x00422c30"],
    "state_8_stage_results_replay": ["0x00438100", "0x00423260"],
    "state_9_school_exit": ["0x004384a0", "0x00423380"],
    "state_10_pause_overlay": ["0x00438970", "0x004233e0"],
    "state_11_post_race_dispatch": ["0x00438b90", "0x00423420"],
    "state_12_quit_funnel": ["0x004398a0", "0x004234a0"],
    "orchestrator_winmain": ["0x0049db00"],
    "orchestrator_main_loop": ["0x004c1e00"],
    "orchestrator_mechanics_25hz": ["0x0043afd0"],
    "orchestrator_render_pose": ["0x00439ee0"],
}

# Curated overrides for known partial / implemented_unwired / non_core_omitted items
# grounded in docs/PRESERVATION_AUDIT.md and live code inspection.
CURATED_OVERRIDES: dict[str, dict[str, str]] = {
    # State 4 loading progress bar: LoadProgress wired end-to-end via StageLoadPump
    "0x00421da0": {
        "status": "verified_1to1",
        "notes": "LoadInterstitial steps LoadProgress (0..=0x1a) across State-4 frames via StageLoadPump and draws BAR/LINE over the target stage .LOA.",
    },
    # Replay recording & post-stage State-8 playback
    "0x0046d8f0": {
        "status": "verified_1to1",
        "notes": "Live State-5 RLE input recording wired in GameplaySession (StageReplayRecorder).",
    },
    "0x0046dd50": {
        "status": "verified_1to1",
        "notes": "Per-tick replay playback wired for both Rally School demos (States 7/9) and live State-8 post-stage results replay.",
    },
    "0x0046c7d0": {
        "status": "verified_1to1",
        "notes": "Stage-start and periodic 50-tick mid-stage RaceState seek keyframes wired in StageReplayRecorder (GameplaySession::seek_stage_replay / FUN_0046ca70).",
    },
    "0x00438100": {
        "status": "verified_1to1",
        "notes": "State-8 stage results update runs results UI over live looping post-stage replay playback.",
    },
    # State-5 physics & render items audited against disassembly
    "0x00415d20": {
        "status": "verified_1to1",
        "notes": "Scales 4 response words by timestep dt_q16 (DAT_004eac6c = 1.0 at 25 Hz); reconstructed as vehicle_dynamics::scale_contact_response_words.",
    },
    "0x004102e0": {
        "status": "verified_1to1",
        "notes": "Arms righting animation (ctl +0x3ec, 20-tick roof/side counters +0x43c..+0x43e); reconstructed in cmr-core::tumble and wired at race_state.rs.",
    },
    "0x00409f90": {
        "status": "verified_1to1",
        "notes": "Builds 8 contact basis vectors (+0x4bc..+0x518) with modular-negation mirror and velocity add; reconstructed as vehicle_dynamics::build_contact_basis_vectors.",
    },
    "0x00439ee0": {
        "status": "verified_1to1",
        "notes": "Audited 0x43a200..0x43ad10: poses root car, Node-5 body roll/heave, 4 wheels, camera, .moj/.obj moving objects, shadow, lamps; remaining branches are cheat scaling/hovercraft.",
    },
    # In-race pacenote corner arrows (images/osdnew/arrws.bmp)
    "0x004442b0": {
        "status": "verified_1to1",
        "notes": "Visual pacenote arrow FIFO push (PacenoteArrowHud::push_number).",
    },
    "0x00444320": {
        "status": "verified_1to1",
        "notes": "Visual pacenote arrow direction row bump (+9 for left, PacenoteArrowHud::accumulate_direction).",
    },
    "0x004619e0": {
        "status": "verified_1to1",
        "notes": "Visual pacenote arrow dismiss + FIFO shift (PacenoteArrowHud::dismiss + 12-waypoint TTL).",
    },
    "0x004625d0": {
        "status": "verified_1to1",
        "notes": "In-race pacenote corner arrow slide animation & 64x64 tile renderer (PacenoteArrowHud + plan_pacenote_arrow_hud, flags 0x08 right / 0x01 left flip).",
    },
    "0x0045f530": {
        "status": "verified_1to1",
        "notes": "Normal single-player race HUD rev strip, speed, gear, timer, progress rail, and GAME DISPLAYS (OFF/MINIMAL/FULL) gates (race_hud_plan.rs + compose_live_race_hud_with_countdown).",
    },
    "0x0045feb0": {
        "status": "verified_1to1",
        "notes": "Classic analog dial HUD bitmap loader & 640x480 rect descriptors (640x480a/b/c.bmp + needle.bmp in RaceHudAssets + plan_classic_single_player_race_hud).",
    },
    "0x00460c40": {
        "status": "verified_1to1",
        "notes": "Classic analog dial HUD renderer with rotated tachometer needle for Classic/Group-B cars 3/4/5/8 (plan_classic_single_player_race_hud + draw_classic_tachometer_needle).",
    },
    # Falling rain streaks and snow billboards (IMAGES\SNOW.BMP)
    "0x0048b960": {
        "status": "verified_1to1",
        "notes": "Stage weather/precipitation init (loads IMAGES\\SNOW.BMP, configures fog and PrecipitationSystem).",
    },
    "0x0048bc00": {
        "status": "verified_1to1",
        "notes": "Precipitation random table init (GameLcg in PrecipitationSystem::tick).",
    },
    "0x0048bc60": {
        "status": "verified_1to1",
        "notes": "Precipitation particle pool init (50/100/200 particles in 12m box 10m ahead of camera, PrecipitationSystem::new/tick).",
    },
    "0x0048c080": {
        "status": "verified_1to1",
        "notes": "Periodic precipitation intensity variation and light-weather lulls (PrecipitationSystem::tick).",
    },
    "0x0048c230": {
        "status": "verified_1to1",
        "notes": "Smooth wind speed/direction slewing plus gravity (PrecipitationSystem::tick).",
    },
    "0x0048c620": {
        "status": "verified_1to1",
        "notes": "Rain streak & SNOW.BMP snowflake advection, box wrapping, night attenuation, and quad emission (PrecipitationSystem::billboards + StageRenderResources::plan_particles).",
    },
    # Sky sun, halo, 7 lens-flare rings, moon, and twinkling stars (IMAGES\LENSFLR\*)
    "0x00482d10": {
        "status": "verified_1to1",
        "notes": "Loads images\\lensflr\\sun.bmp, sunhalo.bmp, and 1.bmp..7.bmp (SkyOverlayAssets::load).",
    },
    "0x00483060": {
        "status": "verified_1to1",
        "notes": "Projects world sun/moon/star direction into screen space (SkyOverlayState::project_dir).",
    },
    "0x00483a40": {
        "status": "verified_1to1",
        "notes": "Dispatches sun disc, sun halo, and lens flare ring sprites (SkyOverlayState::plan).",
    },
    "0x00483c00": {
        "status": "verified_1to1",
        "notes": "Draws sun disc bitmap additively (SkyOverlaySpriteKind::Sun + composite_sky_overlay).",
    },
    "0x00483cc0": {
        "status": "verified_1to1",
        "notes": "Draws sun halo bitmap additively (SkyOverlaySpriteKind::SunHalo + composite_sky_overlay).",
    },
    "0x00483db0": {
        "status": "verified_1to1",
        "notes": "Draws 7 lens-flare rings along sun-to-center axis (SkyOverlaySpriteKind::LensFlare + composite_sky_overlay).",
    },
    "0x00484130": {
        "status": "verified_1to1",
        "notes": "Loads MOON.BMP and STAR.BMP and seeds 60 upper-dome stars (SkyOverlayAssets::load + SkyOverlayState::new).",
    },
    "0x00484a70": {
        "status": "verified_1to1",
        "notes": "Projects and draws night moon sprite (SkyOverlaySpriteKind::Moon + composite_sky_overlay).",
    },
    "0x00484eb0": {
        "status": "verified_1to1",
        "notes": "Night sky dispatcher projecting 60 twinkling stars and moon (SkyOverlayState::plan, gated by config[0x10] & 0x40).",
    },
    "0x00485100": {
        "status": "verified_1to1",
        "notes": "Draws twinkling star sprites additively (SkyOverlaySpriteKind::Star + composite_sky_overlay).",
    },
    "0x00433750": {
        "status": "implemented_unwired",
        "notes": "Force-feedback effect update reconstructed in cmr-core::force_feedback; SDL rumble approximates it.",
    },
    "0x00442740": {
        "status": "implemented_unwired",
        "notes": "Car-vs-car OBB collision reconstructed in cmr-core::car_vs_car_collision; not active in 1P.",
    },
    # Non-core / legacy modes (2P split-screen, LAN DirectPlay, ghost, cheats)
    "0x0041b110": {
        "category": "core_game",
        "status": "non_core_omitted",
        "notes": "2-player split-screen control preparation.",
    },
    "0x00459db0": {
        "category": "core_game",
        "status": "non_core_omitted",
        "notes": "DirectPlay LAN multiplayer state sync.",
    },
    "0x0045a7f0": {
        "category": "core_game",
        "status": "non_core_omitted",
        "notes": "DirectPlay LAN multiplayer dead-reckoning.",
    },
    "0x0047a900": {
        "category": "core_game",
        "status": "non_core_omitted",
        "notes": "Time-Trial ghost car tick (0x428320) & cheat car tick (0x484510).",
    },
    "0x00484950": {
        "category": "core_game",
        "status": "non_core_omitted",
        "notes": "Special cheat (+0x25 & 0x80) car animation interpolation.",
    },
}

VALID_CATEGORIES = {
    "crt",
    "zlib",
    "import_thunk",
    "platform_wrapper",
    "core_game",
}

VALID_STATUSES = {
    "excluded",            # crt / zlib / import_thunk
    "platform_replaced",   # platform_wrapper replaced by SDL3/wgpu/std
    "verified_1to1",       # explicitly audited 1:1 and live-wired
    "wired_live",          # cited in non-test Rust code reachable from production crates
    "partial",             # live-wired with known missing tail/branch/callees
    "implemented_unwired", # implemented in cmr-core/video/formats or test-only, not wired into cmr-app
    "unmapped",            # not referenced in crates/ at all
    "non_core_omitted",    # explicitly deferred legacy mode (2P split-screen, DirectPlay, etc.)
}


def classify_address(addr: int, name: str, size: int) -> str:
    if addr >= 0x004CFAB0:
        return "crt"
    if 0x004CCDC0 <= addr <= 0x004CF8E0:
        return "zlib"
    if 0x004B8760 <= addr <= 0x004B881F or (size <= 6 and name.startswith("thunk_")):
        return "import_thunk"
    # WinMM MCI AVI wrappers
    if 0x004AF8A0 <= addr <= 0x004AFE5F:
        return "platform_wrapper"
    # DirectDraw / D3D2 / Win32 / DirectInput / QMixer / Registry low-level wrappers
    if 0x00495400 <= addr <= 0x004A0FFF:
        return "platform_wrapper"
    if 0x004AFE60 <= addr <= 0x004B376F:
        return "platform_wrapper"
    return "core_game"


def scan_rust_references() -> dict[str, dict[str, list[str]]]:
    """Scan crates/**/*.rs for 0x004xxxxx / FUN_004xxxxx references, split by live vs test-only."""
    pat = re.compile(
        r"(?:\bFUN_00(4[0-9a-fA-F]{5})\b|\b0x00(4[0-9a-fA-F]{5})\b|\b0x(4[0-9a-fA-F]{5})\b|\b0x00(4[0-9a-fA-F])_([0-9a-fA-F]{4})\b)"
    )
    refs: dict[str, dict[str, set[str]]] = defaultdict(lambda: {"live": set(), "test": set()})

    for path in sorted((REPO_ROOT / "crates").rglob("*.rs")):
        rel = path.relative_to(REPO_ROOT).as_posix()
        text = path.read_text(errors="ignore")
        is_test_file = "/tests/" in rel or rel.endswith("_test.rs")
        # Split at `#[cfg(test)]\nmod tests` if present in a non-test file
        if is_test_file:
            live_part, test_part = "", text
        else:
            m = re.search(r"^\s*#\[cfg\(test\)\]\s*\n\s*(?:pub\s+)?mod\s+\w+", text, re.MULTILINE)
            if m:
                live_part, test_part = text[: m.start()], text[m.start() :]
            else:
                live_part, test_part = text, ""

        for bucket, chunk in (("live", live_part), ("test", test_part)):
            for match in pat.finditer(chunk):
                if match.group(1):
                    hex_addr = match.group(1)
                elif match.group(2):
                    hex_addr = match.group(2)
                elif match.group(3):
                    hex_addr = match.group(3)
                else:
                    hex_addr = match.group(4) + match.group(5)
                addr_str = f"0x00{hex_addr.lower()}"
                refs[addr_str][bucket].add(rel)

    return {
        addr: {"live": sorted(d["live"]), "test": sorted(d["test"])}
        for addr, d in refs.items()
    }


def extract_call_graph(proc_addrs: list[int]) -> tuple[dict[int, set[int]], dict[int, set[int]]]:
    """Extract direct `.text` call edges from .work/rea/rally-text.disasm."""
    callees: dict[int, set[int]] = {a: set() for a in proc_addrs}
    callers: dict[int, set[int]] = {a: set() for a in proc_addrs}
    proc_set = set(proc_addrs)

    # Matches objdump intel format: `  43afd5:\te8 16 01 00 00       \tcall   0x43b0f0`
    line_re = re.compile(
        r"^\s*([0-9a-fA-F]+):\s+(?:[0-9a-fA-F]{2}\s+)+\s*(call|jmp)\s+(?:0x)?([0-9a-fA-F]+)\b"
    )
    with DISASM_PATH.open("r", errors="ignore") as f:
        for line in f:
            m = line_re.match(line)
            if not m:
                continue
            insn_addr = int(m.group(1), 16)
            mnemonic = m.group(2)
            target = int(m.group(3), 16)
            if target not in proc_set:
                continue
            idx = bisect.bisect_right(proc_addrs, insn_addr) - 1
            if idx < 0:
                continue
            src = proc_addrs[idx]
            # Treat `call` and cross-function tail `jmp` as call-graph edges.
            if target != src and (mnemonic == "call" or target != src):
                if mnemonic == "call" or (mnemonic == "jmp" and target in proc_set and abs(target - src) > 0):
                    # Only record `jmp` as a tail-call if target is a distinct procedure entry
                    # (not an intra-function jump, which wouldn't be in proc_set anyway).
                    callees[src].add(target)
                    callers[target].add(src)

    return callees, callers


def generate_map() -> None:
    if not PROCS_PATH.exists() or not DISASM_PATH.exists():
        sys.exit(
            f"error: --generate requires {PROCS_PATH.relative_to(REPO_ROOT)} and "
            f"{DISASM_PATH.relative_to(REPO_ROOT)}"
        )

    existing_overrides: dict[str, dict] = {}
    if MAP_PATH.exists():
        old = json.loads(MAP_PATH.read_text())
        for p in old.get("procedures", []):
            if p.get("manual_override"):
                existing_overrides[p["address"]] = p

    raw_procs = json.loads(PROCS_PATH.read_text())["normalized_result"]
    text_proc_map: dict[int, str] = {}
    for p in raw_procs:
        addr_s = p.get("address", "")
        if addr_s.startswith("EXTERNAL:"):
            continue
        addr = int(addr_s, 16)
        if TEXT_START <= addr < TEXT_END:
            text_proc_map[addr] = p.get("value", f"FUN_{addr:08x}")

    # Include indirect-table FSM handler entries (`0x4ee82c`) not carved by Ghidra.
    for roots in FSM_ROOTS.values():
        for r in roots:
            addr = int(r, 16)
            if addr not in text_proc_map:
                text_proc_map[addr] = f"FUN_{addr:08x}"

    text_procs = sorted(text_proc_map.items(), key=lambda x: x[0])
    proc_addrs = [a for a, _ in text_procs]
    callees_map, callers_map = extract_call_graph(proc_addrs)
    rust_refs = scan_rust_references()

    procedures = []
    for i, (addr, name) in enumerate(text_procs):
        end_addr = proc_addrs[i + 1] if i + 1 < len(proc_addrs) else TEXT_END
        size = end_addr - addr
        addr_s = f"0x{addr:08x}"
        category = classify_address(addr, name, size)

        refs = rust_refs.get(addr_s, {"live": [], "test": []})
        live_files = refs["live"]
        test_files = refs["test"]

        if category in ("crt", "zlib", "import_thunk"):
            status = "excluded"
        elif category == "platform_wrapper":
            status = "platform_replaced"
        elif live_files:
            status = "wired_live"
        elif test_files:
            status = "implemented_unwired"
        else:
            status = "unmapped"

        notes = ""
        manual_override = False

        if addr_s in existing_overrides:
            ov = existing_overrides[addr_s]
            category = ov.get("category", category)
            status = ov.get("status", status)
            notes = ov.get("notes", notes)
            manual_override = True

        if addr_s in CURATED_OVERRIDES:
            ov = CURATED_OVERRIDES[addr_s]
            if "category" in ov:
                category = ov["category"]
            if "status" in ov:
                status = ov["status"]
            if "notes" in ov:
                notes = ov["notes"]
            manual_override = True

        entry = {
            "address": addr_s,
            "end_address": f"0x{end_addr:08x}",
            "size": size,
            "name": name,
            "category": category,
            "status": status,
            "callees": [f"0x{c:08x}" for c in sorted(callees_map[addr])],
            "callers": [f"0x{c:08x}" for c in sorted(callers_map[addr])],
            "live_files": live_files,
            "test_files": test_files,
        }
        if notes:
            entry["notes"] = notes
        if manual_override:
            entry["manual_override"] = True
        procedures.append(entry)

    out = {
        "schema_version": 1,
        "binary": {
            "name": "Rally.exe",
            "edition": "official-eu",
            "sha256": "0eb6ea9652219b9a35c0a6134bf0cdf316dce21215b724cdfc10ba85eb95982a",
            "text_range": [f"0x{TEXT_START:08x}", f"0x{TEXT_END:08x}"],
            "text_procedure_count": len(procedures),
        },
        "fsm_roots": FSM_ROOTS,
        "procedures": procedures,
    }
    MAP_PATH.write_text(json.dumps(out, indent=2) + "\n")
    print(f"wrote {MAP_PATH.relative_to(REPO_ROOT)} ({len(procedures)} .text procedures)")


def bfs_reachable(roots: list[str], by_addr: dict[str, dict]) -> set[str]:
    visited: set[str] = set()
    q: deque[str] = deque()
    for r in roots:
        if r in by_addr and r not in visited:
            visited.add(r)
            q.append(r)
    while q:
        cur = q.popleft()
        for nxt in by_addr[cur]["callees"]:
            if nxt in by_addr and nxt not in visited:
                visited.add(nxt)
                q.append(nxt)
    return visited


def verify_and_report(print_report: bool, state_filter: str | None) -> int:
    if not MAP_PATH.exists():
        print(f"error: missing {MAP_PATH}", file=sys.stderr)
        return 1

    data = json.loads(MAP_PATH.read_text())
    procs = data.get("procedures", [])
    errors: list[str] = []

    if len(procs) != EXPECTED_TEXT_PROCS:
        errors.append(
            f"expected {EXPECTED_TEXT_PROCS} .text procedures, found {len(procs)}"
        )

    by_addr: dict[str, dict] = {}
    prev_addr = -1
    for p in procs:
        addr_s = p["address"]
        addr = int(addr_s, 16)
        if addr <= prev_addr:
            errors.append(f"procedures not strictly sorted at {addr_s}")
        prev_addr = addr
        if p["category"] not in VALID_CATEGORIES:
            errors.append(f"invalid category {p['category']!r} at {addr_s}")
        if p["status"] not in VALID_STATUSES:
            errors.append(f"invalid status {p['status']!r} at {addr_s}")
        by_addr[addr_s] = p

    # Verify live_files / test_files actually exist on disk and catch broken paths.
    for p in procs:
        for f in p.get("live_files", []) + p.get("test_files", []):
            if not (REPO_ROOT / f).exists():
                errors.append(f"{p['address']}: referenced Rust file does not exist: {f}")

    # Verify every verified_1to1 / wired_live entry has at least one live Rust reference.
    for p in procs:
        if p["status"] in ("verified_1to1", "wired_live") and not p.get("live_files"):
            errors.append(
                f"{p['address']} ({p['name']}): status={p['status']} but has no live_files"
            )

    if errors:
        print(f"function-map verification FAILED ({len(errors)} errors):", file=sys.stderr)
        for e in errors[:25]:
            print(f"  - {e}", file=sys.stderr)
        return 1

    # Summary statistics
    cat_counts: dict[str, int] = defaultdict(int)
    core_status_counts: dict[str, int] = defaultdict(int)
    for p in procs:
        cat_counts[p["category"]] += 1
        if p["category"] == "core_game":
            core_status_counts[p["status"]] += 1

    total_core = cat_counts["core_game"]
    print(
        f"function-map verified: {len(procs)} .text procedures "
        f"(core_game={total_core}, platform_wrapper={cat_counts['platform_wrapper']}, "
        f"crt={cat_counts['crt']}, zlib={cat_counts['zlib']}, import_thunk={cat_counts['import_thunk']})"
    )
    print(
        "  core_game status: "
        + ", ".join(
            f"{k}={core_status_counts[k]}"
            for k in (
                "verified_1to1",
                "wired_live",
                "partial",
                "implemented_unwired",
                "non_core_omitted",
                "unmapped",
            )
            if core_status_counts[k]
        )
    )

    if print_report or state_filter is not None:
        print("\n=== Per-FSM-State & Orchestrator Reachability Report (core_game only) ===")
        for label, roots in data.get("fsm_roots", {}).items():
            if state_filter is not None and state_filter not in label:
                continue
            reach = bfs_reachable(roots, by_addr)
            core_reach = [by_addr[a] for a in sorted(reach) if by_addr[a]["category"] == "core_game"]
            counts: dict[str, int] = defaultdict(int)
            for p in core_reach:
                counts[p["status"]] += 1
            print(
                f"  {label:<32} total={len(core_reach):4d} | "
                f"1:1={counts['verified_1to1']:3d} "
                f"live={counts['wired_live']:3d} "
                f"partial={counts['partial']:2d} "
                f"unwired={counts['implemented_unwired']:2d} "
                f"omitted={counts['non_core_omitted']:2d} "
                f"unmapped={counts['unmapped']:3d}"
            )
            if state_filter is not None:
                for p in core_reach:
                    if p["status"] in ("partial", "implemented_unwired"):
                        print(f"    [{p['status']}] {p['address']} {p['name']}: {p.get('notes', '')}")
                # Direct callees of the state roots that are unmapped/partial/unwired
                print("    Direct root callees with gaps:")
                for r in roots:
                    if r not in by_addr:
                        continue
                    for c in by_addr[r]["callees"]:
                        cp = by_addr[c]
                        if cp["category"] == "core_game" and cp["status"] not in ("verified_1to1", "wired_live"):
                            print(f"      {r} -> {c} [{cp['status']}] {cp.get('notes', '')}")

        print("\n=== Curated Known Gaps (partial & implemented_unwired) ===")
        for p in procs:
            if p["status"] in ("partial", "implemented_unwired"):
                print(f"  [{p['status']:<19}] {p['address']} ({p['name']}): {p.get('notes', '(test-only reference)')}")

    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generate", action="store_true", help="Regenerate docs/function-map.json")
    parser.add_argument("--verify", action="store_true", help="Verify docs/function-map.json")
    parser.add_argument("--report", action="store_true", help="Print per-FSM-state reachability report")
    parser.add_argument("--state", type=str, default=None, help="Filter report to matching FSM state label")
    args = parser.parse_args()

    if args.generate:
        generate_map()
    return verify_and_report(print_report=args.report, state_filter=args.state)


if __name__ == "__main__":
    raise SystemExit(main())
