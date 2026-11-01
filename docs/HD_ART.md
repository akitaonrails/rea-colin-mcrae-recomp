# Optional HD art pack

The modern branch can reuse the compact content-addressed HD texture pack made
by the sibling `rea-colin-mcrae` project. The pack is optional and disabled by
default. Its generated images are local user data and must not be committed.

Install a pack by placing its uncompressed compact folder here:

```text
<runtime-data>/hd-pack/pack.txt
<runtime-data>/hd-pack/x2/<16-hex-hash>.png-or-webp
<runtime-data>/hd-pack/x4/<16-hex-hash>.png-or-webp
```

Enable it in **Settings → Graphics → Advanced → HD ART PACK**. Output up to
1920x1080 selects `x2`; larger output selects `x4`. Changing the toggle or the
resolution rebuilds retained stage, scenery, object, horizon, and car C3D
textures without weakening the original compatibility path.

Indexed PNG replacements preserve their source indices and palette. True-colour
PNG/WebP replacements are deterministically reduced to an adaptive 256-colour
palette at their full x2/x4 dimensions; palette index zero remains reserved for
alpha-keyed black so the existing indexed renderer and colour-key contract stay
intact.

Each replacement is selected by FNV-1a-64 of the complete original BMP bytes,
not by a game path. A different game edition therefore cannot accidentally
receive a replacement for unlike bytes. Missing, unreadable, oversized, or
malformed pack entries fail open to the authenticated original texture.
Successful activation is reported on stderr with the selected scale and the
number of replacements installed.

For this workspace, the approved local pack was copied to the ignored
`runtime-data/installed-eu-derived-assets-v2/hd-pack/` directory. The source
release ZIP has SHA-256
`ff5044ff37b9c23a4c09260ed9bffbb7424acdb66e50c5147daff964a261dda8`.
