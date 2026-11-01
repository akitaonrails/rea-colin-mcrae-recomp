# Colin McRae Rally — Modern Reconstruction

This package contains independently written reconstruction binaries and
reviewed metadata profiles. It does **not** contain the original game or any
game assets. You must supply a legally owned European PC installation.

## Linux AppImage

Make the file executable, then verify its version:

```sh
chmod +x Colin-McRae-Rally-Modern-x86_64.AppImage
./Colin-McRae-Rally-Modern-x86_64.AppImage --version
```

The importer is available through `--asset-tool`. Relative profile paths are
resolved inside the AppImage; use absolute paths for your source and output:

```sh
./Colin-McRae-Rally-Modern-x86_64.AppImage --asset-tool verify \
  --source /absolute/path/to/CMRally \
  --profile profiles/installed-eu-derived-assets-v2.json

./Colin-McRae-Rally-Modern-x86_64.AppImage --asset-tool import \
  --source /absolute/path/to/CMRally \
  --profile profiles/installed-eu-derived-assets-v2.json \
  --output /absolute/path/to/runtime-data/installed-eu-derived-assets-v2
```

The executable-backed table extraction commands listed in the project README
must also be run once. Launch the game with the same arguments documented in
the README, using absolute paths.

## Windows ZIP

Extract the complete ZIP before running it; keep `SDL3.dll` beside the two
executables. Run `cmr-assets.exe` from PowerShell or Command Prompt for the
one-time import, then launch `cmr-app-modern.exe` with the documented game
session arguments.

`--renderer auto` is the recommended default on both platforms. It discovers
available native graphics APIs and compatible adapters at runtime, including
hybrid/multi-GPU systems, then falls back to the software renderer if no GPU
path can be created. Windowed resolutions are fitted to the active display's
reported DPI and usable desktop area; no release setting is tied to the build
machine's monitors.

## Integrity and local data

Compare downloads against `SHA256SUMS.txt` on the GitHub release. Imported data,
save files, screenshots, and an optional HD pack remain local and are never
written back into these read-only release artifacts.

The initial binaries are not platform-code-signed. Windows may display a
SmartScreen warning; verify the ZIP against `SHA256SUMS.txt` before running it.
