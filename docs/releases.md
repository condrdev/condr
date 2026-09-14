# Releases

Condr publishes two kinds of builds. `nightly.yml` and `release.yml` both call the reusable `build.yml`, which packages the GUI + CLI installer and the CLI-only archive for Linux x86_64/arm64, macOS x86_64/arm64 and Windows x86_64, all from one commit, with `SHA256SUMS` and a `BUILD-COMMIT` file inside each package. Client and Server have no cross-build compatibility promise; update them together.

## Nightly

- Runs daily at 03:17 UTC and on demand (`gh workflow run nightly.yml`).
- Builds the current `main`. If the `nightly` tag already points at `HEAD`, the scheduled run exits without building.
- Recreates the mutable **Nightly** prerelease and its `nightly` tag at the built commit, so only assets from that commit are attached.
- Package names carry the 12-character short SHA: `condr-0.1.0-<sha>-linux-x86_64.AppImage`, `condr-cli-0.1.0-<sha>-macos-arm64.tar.gz`.
- `nightly` is the only mutable tag, and only the workflow recreates it.
- The workflow creates the tag with the built-in `GITHUB_TOKEN`, which GitHub refuses when the target commit's `.github/workflows/` files differ from `main`. In practice that only happens when a workflow change lands on `main` while a nightly is still building; the publish step then fails with HTTP 403 and the next run succeeds.

## Versioned release

1. Bump `version` in the root `Cargo.toml` and merge it to `main`.
2. Tag the merged commit and push the tag:

   ```bash
   git tag v0.1.0 <commit>
   git push origin v0.1.0
   ```

3. The workflow verifies the tag matches `Cargo.toml`, builds with `CONDR_RELEASE=1` (package names without the SHA), and creates an immutable release marked **Latest** with generated notes. It fails instead of overwriting an existing release.

## Installing a build

Desktop users download the installer from [Releases](https://github.com/condrdev/condr/releases). Headless servers use the install scripts, which fetch the CLI archive for the current machine through the GitHub CLI:

```bash
sh script/install-condr.sh                      # latest nightly
CONDR_VERSION=v0.1.0 sh script/install-condr.sh # a versioned release
sh script/install-condr.sh --from ./condr-cli-<version>-linux-x86_64.tar.gz
```

```powershell
powershell -ExecutionPolicy Bypass -File script\install-condr.ps1              # latest nightly
powershell -ExecutionPolicy Bypass -File script\install-condr.ps1 -Version v0.1.0
```

Package layout, data directories and per-platform packaging scripts are described in [development-build.md](development-build.md).
