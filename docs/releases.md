# Releases

Condr publishes two kinds of builds. `nightly.yml` and `release.yml` both call the reusable `build.yml`, which packages the desktop installer and the headless archive for Linux x86_64/arm64, macOS arm64 and Windows x86_64, all from one commit, with `SHA256SUMS` and a `BUILD-COMMIT` file inside each package. The Companion, the iPhone and Android app, has its own version and release (ADR 0049, [below](#companion-release)); the nightly is the desktop's alone. Client and Server have no cross-build compatibility promise; update them together. Every binary reports its build as `<version>+<12-character commit>` (`condr --version`, `condr-gui --version`; ADR 0027), a nightly with `(nightly)` after it, and a GUI connected to a Server of another build marks that Device in the sidebar.

Intel Macs get no package: Apple has ended the architecture, GitHub's last x86_64 macOS runner retires in 2027, and stapling its notarization kept failing there. The Windows GUI ships as its installer alone, with no portable ZIP; the headless ZIP stays, since `install.ps1` installs the CLI from it.

## Nightly

- Runs daily at 03:17 UTC and on demand (`gh workflow run nightly.yml`).
- Builds the current `main`. If the `nightly` tag already points at `HEAD`, the scheduled run exits without building.
- Recreates the mutable **Nightly** prerelease and its `nightly` tag at the built commit, so only assets from that commit are attached.
- Its notes list the `feat` and `fix` commits since the latest `v*` tag, the same way a versioned release does (without the Update section), and link the compare view of every commit since.
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

3. The workflow verifies the tag matches `Cargo.toml`, builds with `CONDR_RELEASE=1` (package names without the SHA), and creates a release marked **Latest**. It fails instead of overwriting an existing release, so a published version's packages never change. GitHub's immutable releases setting stays off: `nightly.yml` deletes the Nightly release and recreates it under the same `nightly` tag, which an immutable release forbids.

The release notes come from the commits since the previous `v*` tag: [git-cliff](https://git-cliff.org) with `cliff.toml` lists `feat` commits under New and `fix` commits under Fixed, by subject, skips every other type and `feat(website)`, and ends with the Update section. `release.yml` and `nightly.yml` leave out a commit whose every file is the Companion's (`mobile/`, `crates/condr-mobile/` and its two scripts) or documentation (`docs/`, `website/`, `.github/`, Markdown, `.gitignore`); one that touches the desktop too is in both, and the Companion's own changes are in its notes. So a `feat` or `fix` subject is a line of the release notes. Preview them before tagging with `git cliff --unreleased --tag v<next> --strip header` and the `--exclude-path` list `release.yml` passes; a summary sentence or a note that only the new Server has something can be added by editing the published release.

## Companion release

The iPhone and Android app share one version, `MARKETING_VERSION` in `mobile/version.xcconfig`, released apart from the desktop (ADR 0049). Android's `versionCode` and the iPhone's build number follow from it, so the version is the only number to change; every release bumps it, even one that changes a single platform.

1. Bump `MARKETING_VERSION` in `mobile/version.xcconfig` and merge it to `main`.
2. Tag the merged commit and push the tag:

   ```bash
   git tag companion-v0.1.15 <commit>
   git push origin companion-v0.1.15
   ```

3. `companion.yml` checks the tag against the file, builds the APK, signed with the upload key it will not go without, and publishes it with `SHA256SUMS` as **Condr Companion 0.1.15**, never marked Latest, and archives the iPhone app and uploads it to TestFlight. The APK is published only once the iPhone build is uploaded, so the two ship together, and then condr.dev redeploys so its download page links that APK. In App Store Connect, add the processed build to the public group. Every Companion release is a new version, and a version's first build for external testers goes through Beta App Review, usually within a day.

The notes take the commits since the previous `companion-v*` tag (the first release starts from `v0.1.14`) that touch `mobile/`, `crates/condr-mobile/`, `crates/condr-client/`, which both apps link, or the two mobile scripts. Preview them with the command `companion.yml` runs, `--tag companion-v<next>` and all. `gh workflow run companion.yml`, on any branch or tag, builds and signs both platforms and publishes nothing, which checks the secrets. The Android job also keeps the Play bundle, `condr-companion-<version>.aab`, as the `condr-companion-bundle` artifact for seven days; upload it to a testing track in Play Console.

The iPhone upload signs with an App Store Connect team key that has the **Admin** role, so `xcodebuild -allowProvisioningUpdates` may create the cloud-managed distribution certificate and the App Store profile; the notary key's Developer role cannot. Create it under Users and Access › Integrations › App Store Connect API › Team Keys. The archive is signed for development first, so the job also imports one Apple Development certificate kept for CI; without it, every run would have the API create a new one. That certificate was made through the API from a key that never entered a keychain, and it expires on 2027-10-23; renew it the same way, as a `DEVELOPMENT` certificate from a new CSR, and revoke the old one.

| Secret | Value |
| --- | --- |
| `APPLE_ASC_KEY` | The Admin key's `.p8`, base64-encoded |
| `APPLE_ASC_KEY_ID` | Its Key ID |
| `APPLE_NOTARY_ISSUER` | The team's Issuer ID, shared with the notary key |
| `APPLE_DEVELOPMENT_P12` | The CI's Apple Development certificate with its private key, `.p12`, base64-encoded |
| `APPLE_DEVELOPMENT_P12_PASSWORD` | The password of that `.p12` |
| `ANDROID_KEYSTORE`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_PASSWORD` | The Android key: it signs the APK and the Play bundle, and Google Play signs with it too (ADR 0048) |

## macOS signing and notarization

`build.yml` signs and notarizes the macOS packages, nightlies and releases alike, when the repository holds these secrets; without them (a fork, or before they are set) the packages are ad-hoc signed and Gatekeeper asks the user to allow the app once.

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE_P12` | The Developer ID Application certificate with its private key, exported from Keychain Access as `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | The password given at that export |
| `APPLE_NOTARY_KEY` | An App Store Connect team API key (`.p8`, Developer role or above), base64-encoded |
| `APPLE_NOTARY_KEY_ID` | Its Key ID |
| `APPLE_NOTARY_ISSUER` | The Issuer ID shown with the team's keys |

The macOS job imports the certificate into a temporary keychain, reads the identity's name from it, writes the key to a file, and hands them to `script/package-macos.sh` as `CONDR_SIGNING_IDENTITY` and `CONDR_NOTARY_KEY`; `check-macos-package.sh` then holds the packages to Gatekeeper's verdict. The certificate is valid for five years, and the trusted timestamp keeps packages signed before it expires valid. Windows packages are not signed; SmartScreen needs a separate certificate. How the packaging script signs is described in [development-build.md](development-build.md).

## Installing a build

Desktop users download the installer from [Releases](https://github.com/condrdev/condr/releases). Headless servers use `script/install-condr.sh` / `.ps1`, which condr.dev serves as `install.sh` / `install.ps1`: the script picks the headless archive for the current machine from GitHub Releases, verifies it against `SHA256SUMS`, and runs `condr server install` with any remaining arguments (ADR 0016). Asked for nothing else, it first compares the installed `condr --version` with `condr.dev/version.txt` (the Cargo.toml version the site was built from, redeployed by `release.yml`; the GUI's stable update check reads it too) and stops without downloading when that release is already installed; `--force` reinstalls.

```bash
curl -fsSL https://condr.dev/install.sh | sh                          # the latest release
curl -fsSL https://condr.dev/install.sh | CONDR_VERSION=nightly sh    # the nightly
curl -fsSL https://condr.dev/install.sh | CONDR_VERSION=v0.1.0 sh     # one versioned release
curl -fsSL https://condr.dev/install.sh | sh -s -- --start            # also start the Server
curl -fsSL https://condr.dev/install.sh | sh -s -- --force            # reinstall even when up to date
sh script/install-condr.sh --from ./condr-headless-<version>-linux-x86_64.tar.gz
```

```powershell
irm https://condr.dev/install.ps1 | iex                                 # the latest release
$env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
$env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
powershell -ExecutionPolicy Bypass -File script\install-condr.ps1 -From .\condr-headless-<version>-windows-x86_64.zip
```

## Update check

The GUI tells, never installs (ADR 0029). Settings › About has "Check automatically" (`[client.updates] auto_check`, on by default), "Update channel" (`[client.updates] channel`, `stable` or `nightly`; unset follows the build, `CONDR_RELEASE` at compile time, `0` nightly, otherwise stable) and a Check button. Five seconds after start and then every five hours it asks the GitHub API: stable compares the `releases/latest` tag's version with its own, nightly compares the `nightly` tag's commit with its own `CONDR_BUILD_COMMIT`. A newer build puts a dot on the sidebar's Settings button and a row on the About page that opens the release page. Nightly therefore depends on `nightly.yml` keeping the tag on the published commit, and stable on `release.yml` marking each `v*` release Latest.

Package layout, data directories and per-platform packaging scripts are described in [development-build.md](development-build.md).
