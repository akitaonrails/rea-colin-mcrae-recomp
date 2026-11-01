#!/usr/bin/env bash
set -euo pipefail

# I10: the portable reconstruction must not require any of the original game's
# native backends - DirectDraw, DirectInput, DirectSound, QMixer/QSound, or
# DirectPlay. It reaches the host only through the platform/backend traits
# (SDL3 for window/input/audio, wgpu for 3D), so this audit fails if the
# workspace gains a runtime loader, an explicit native link, a dependency that
# binds one of those backends, or a built artifact that imports one.
#
# Note: SDL3/wgpu may use *modern Windows system services* (DXGI/D3D12, and
# winmm timers); that is the replacement backend, not the original game's DLL
# requirement. The audited set below is the legacy game/back-end surface only.

legacy='ddraw|dinput|dsound|dsound3d|qmixer|qswavemix|dplayx|dpmodemx|amstream|quartz'

echo "checking for legacy Windows backend bindings"

# 1. Project code must not load native libraries at runtime, declare native FFI,
#    or link one directly.
if git grep -nIE 'LoadLibrary[A-W]?[[:space:]]*\(|dlopen[[:space:]]*\(|extern[[:space:]]+"|#\[link\(name[[:space:]]*=' -- crates; then
  echo "error: project code must not dynamically load, declare FFI for, or explicitly link native libraries" >&2
  exit 1
fi

# 2. No workspace dependency may bind a legacy backend crate.
if cargo tree --workspace --prefix none 2>/dev/null | grep -iE "$legacy"; then
  echo "error: a workspace dependency binds a legacy Windows backend" >&2
  exit 1
fi

# 3. Any already-built binary must not import one either. Run after building
#    cmr-app (`cargo build --release -p cmr-app`); absent artifacts are skipped.
checked=0
inspected=0
for artifact in \
  target/release/cmr-app \
  target/release/cmr-app.exe \
  target/x86_64-pc-windows-gnu/release/cmr-app.exe; do
  # MSYS2's filesystem predicates transparently try an .exe suffix. Without
  # this guard, `-f target/release/cmr-app` succeeds on GitHub's Windows runner
  # even though only cmr-app.exe exists, and objdump is then given a path that
  # the native Windows executable cannot resolve.
  if [[ "$artifact" != *.exe && -f "${artifact}.exe" ]]; then
    continue
  fi
  [[ -f "$artifact" ]] || continue
  checked=$((checked + 1))
  if [[ "$(head -c 2 "$artifact")" == "MZ" ]]; then
    if ! command -v objdump >/dev/null 2>&1; then
      if [[ ${CMR_REQUIRE_BINARY_INSPECTION:-0} == 1 ]]; then
        echo "error: objdump is required to inspect $artifact" >&2
        exit 1
      fi
      echo "  $(basename "$artifact"): PE import audit skipped (objdump unavailable)"
      continue
    fi
    imports=$(objdump -p "$artifact" | awk '$1 == "DLL" && $2 == "Name" { print $3 }')
  else
    if ! command -v readelf >/dev/null 2>&1; then
      if [[ ${CMR_REQUIRE_BINARY_INSPECTION:-0} == 1 ]]; then
        echo "error: readelf is required to inspect $artifact" >&2
        exit 1
      fi
      echo "  $(basename "$artifact"): ELF import audit skipped (readelf unavailable)"
      continue
    fi
    imports=$(readelf -d "$artifact" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p')
  fi
  inspected=$((inspected + 1))
  if grep -qiE "$legacy" <<<"$imports"; then
    echo "error: $artifact imports a legacy Windows backend:" >&2
    grep -iE "$legacy" <<<"$imports" >&2
    exit 1
  fi
  echo "  $(basename "$artifact"): $(tr '\n' ' ' <<<"$imports")"
done
if ((checked == 0)); then
  echo "  no built cmr-app artifact found; import audit skipped (build one to include it)"
fi
if [[ ${CMR_REQUIRE_BINARY_INSPECTION:-0} == 1 ]] && ((inspected == 0)); then
  echo "error: a built cmr-app artifact is required for release import inspection" >&2
  exit 1
fi

echo "passed: no DirectDraw/DirectInput/DirectSound/QMixer/DirectPlay binding"
