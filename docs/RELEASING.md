# Release process

The project publishes the `modern` branch as data-free x86-64 binaries. The
`main` preservation branch remains available and buildable from source, but is
not mixed into a modern release artifact.

## Artifacts

| Platform | Artifact | Included |
| --- | --- | --- |
| Linux | `Colin-McRae-Rally-Modern-vX.Y.Z-x86_64.AppImage` | `cmr-app`, `cmr-assets`, bundled SDL3, reviewed JSON profiles, release documentation |
| Windows | `Colin-McRae-Rally-Modern-vX.Y.Z-windows-x86_64.zip` | `cmr-app-modern.exe`, `cmr-assets.exe`, `SDL3.dll`, `vcruntime140.dll` (Microsoft's Visual C++ runtime), reviewed JSON profiles, release documentation |

Original executables, media, imported `runtime-data/`, save files, REA output,
and the optional HD pack are forbidden from release artifacts. The workflow
extracts/stages each artifact, rejects known game payload names and formats,
smoke-tests `cmr-app --version`, bundles the registry packages' license texts,
and publishes a combined `SHA256SUMS.txt`.

The initial artifacts are checksum-protected but are not platform-code-signed.
This limitation must remain explicit in the bundled release notes until a
protected signing identity and CI signing boundary are configured.

The executable does not encode a developer monitor or GPU. SDL supplies the
active display's pixel density and usable bounds, and `--renderer auto`
enumerates the platform's available Vulkan, Direct3D 12, and OpenGL adapters.
It filters adapters that lack required texture roles, retries another
compatible adapter if device creation fails, and retains the software renderer
as a path-independent fallback. Forced `--renderer gpu` still fails closed so
GPU problems are diagnosable rather than silently hidden.

The Linux package follows the type-2 AppImage AppDir layout. `appimagetool`
1.9.1 and the x86-64 runtime are downloaded from their upstream AppImage
projects and verified against hashes pinned in
`.github/workflows/release.yml`. SDL3 is built from Cargo's locked source and
its license is included beside the project documentation.

## Candidate gate

1. Confirm `modern` contains `main` and the working tree is clean.
2. Update `CHANGELOG.md`, workspace crate versions, and `Cargo.lock` together.
3. Run the project checks:

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ./scripts/check-portability.sh
   ./scripts/check-legacy-backends.sh
   python -m unittest discover -s scripts/tests -v
   ```

4. Build the bundled-SDL release locally and smoke-test the AppImage.
5. Run the release security/post-merge audit and inspect the staged artifact
   manifest. No ignored local data may enter either artifact.
6. Push `modern` and require both `CI` and `Build Modern Release Artifacts` to
   pass on that exact commit, including native Windows jobs.

## Publish

Create an annotated `vX.Y.Z` tag only after the exact candidate commit is green,
then push that tag. The tag workflow checks out the tag itself, verifies that
the tag matches the `cmr-app` Cargo version, rebuilds both platforms, validates
their contents and checksums, and creates the GitHub release. Never retarget or
force-push a published tag.

After publication, verify the tag and release assets through GitHub, download
and checksum both artifacts, and run the AppImage version smoke test once more.
