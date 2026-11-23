# Colin McRae Rally — Modern Reconstruction

This package contains independently written reconstruction binaries and
reviewed metadata profiles. It does **not** contain the original game or any
game assets. You must supply a legally owned European PC installation.

## Linux AppImage

Make the file executable and launch it directly:

```sh
chmod +x Colin-McRae-Rally-Modern-x86_64.AppImage
./Colin-McRae-Rally-Modern-x86_64.AppImage
```

On first run, a native folder picker asks for your original Colin McRae Rally
(1998, EU v1.5) game folder, imports all assets and lookup tables into
`~/.local/share/cmr-recomp/runtime-data`, and starts the game automatically.
To run setup from a terminal instead:

```sh
./Colin-McRae-Rally-Modern-x86_64.AppImage --asset-tool setup \
  --source /absolute/path/to/CMRally \
  --output ~/.local/share/cmr-recomp/runtime-data
```

## Windows ZIP

Extract the complete ZIP before running it; keep `SDL3.dll`,
`vcruntime140.dll`, and `profiles/` beside `cmr-app-modern.exe`. Double-click
`cmr-app-modern.exe` — on first run, a native folder picker asks for your
original Colin McRae Rally (1998, EU v1.5) game folder, imports all assets and
lookup tables into `runtime-data/`, and starts the game automatically. To run
setup from PowerShell or Command Prompt instead:

```bat
cmr-assets.exe setup --source "C:\path\to\CMRally"
cmr-app-modern.exe
```

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
