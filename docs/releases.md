# Releases

Condr publishes two kinds of builds. `nightly.yml` and `release.yml` both call the reusable `build.yml`, which packages the desktop installer and the headless archive for Linux x86_64/arm64, macOS x86_64/arm64 and Windows x86_64, all from one commit, with `SHA256SUMS` and a `BUILD-COMMIT` file inside each package. Client and Server have no cross-build compatibility promise; update them together. Every binary reports its build as `<version>+<12-character commit>` (`condr --version`, `condr-gui --version`; ADR 0027), and a GUI connected to a Server of another build marks that Device in the sidebar.

## Nightly

- Runs daily at 03:17 UTC and on demand (`gh workflow run nightly.yml`).
- Builds the current `main`. If the `nightly` tag already points at `HEAD`, the scheduled run exits without building.
- Recreates the mutable **Nightly** prerelease and its `nightly` tag at the built commit, so only assets from that commit are attached.
- Package names carry the 12-character short SHA: `condr-0.1.0-<sha>-linux-x86_64.AppImage`, `condr-headless-0.1.0-<sha>-macos-arm64.tar.gz`.
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

Desktop users download the installer from [Releases](https://github.com/condrdev/condr/releases). Headless servers use `script/install-condr.sh` / `.ps1`, which condr.dev serves as `install.sh` / `install.ps1`: the script picks the headless archive for the current machine from GitHub Releases, verifies it against `SHA256SUMS`, and runs `condr server install` with any remaining arguments (ADR 0016).

```bash
curl -fsSL https://condr.dev/install.sh | sh                          # the latest release
CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh    # the nightly
CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh     # one versioned release
curl -fsSL https://condr.dev/install.sh | sh -s -- --start            # also start the Server
sh script/install-condr.sh --from ./condr-headless-<version>-linux-x86_64.tar.gz
```

```powershell
irm https://condr.dev/install.ps1 | iex                                 # the latest release
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
powershell -ExecutionPolicy Bypass -File script\install-condr.ps1 -From .\condr-headless-<version>-windows-x86_64.zip
```

## Update check

The GUI tells, never installs (ADR 0029). Settings › About › Update channel picks `stable`, `nightly` or `off`, stored as `[client] update_channel`; unset follows the build, `CONDR_RELEASE` at compile time (`0` nightly, otherwise stable). Five seconds after start and then daily it asks the GitHub API: stable compares the `releases/latest` tag's version with its own, nightly compares the `nightly` tag's commit with its own `CONDR_BUILD_COMMIT`. A newer build puts a dot on the sidebar's Settings button and a row on the About page that opens the release page. Nightly therefore depends on `nightly.yml` keeping the tag on the published commit, and stable on `release.yml` marking each `v*` release Latest.

Package layout, data directories and per-platform packaging scripts are described in [development-build.md](development-build.md).
