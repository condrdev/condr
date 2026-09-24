# Install the Condr headless build (the condr CLI/Server binary) on Windows x86_64.
#   irm https://condr.dev/install.ps1 | iex
#   $env:CONDR_VERSION = 'nightly'; irm https://condr.dev/install.ps1 | iex
#   $env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
#   $env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
#   $env:CONDR_INSTALL_ARGS = '--force'; irm https://condr.dev/install.ps1 | iex
#   powershell -ExecutionPolicy Bypass -File script\install-condr.ps1 -From .\condr-headless-<version>-windows-x86_64.zip
# Downloads the archive from GitHub Releases (or takes -From), verifies it against
# SHA256SUMS, then runs `condr server install` with the remaining arguments or
# $env:CONDR_INSTALL_ARGS (ADR 0016). With no other arguments it first compares the
# installed condr with condr.dev/version.txt and stops when the latest release is
# already there; --force reinstalls. Desktop users use the installer instead.
param(
  [string] $From,
  [string] $Version,
  [Parameter(ValueFromRemainingArguments)] [string[]] $InstallArgs
)
$ErrorActionPreference = 'Stop'

$repo = if ($env:CONDR_REPO) { $env:CONDR_REPO } else { 'condrdev/condr' }
if (-not $Version) { $Version = if ($env:CONDR_VERSION) { $env:CONDR_VERSION } else { 'latest' } }
if (-not $InstallArgs -and $env:CONDR_INSTALL_ARGS) { $InstallArgs = $env:CONDR_INSTALL_ARGS -split '\s+' }
$force = $InstallArgs -contains '--force'
$InstallArgs = @($InstallArgs | Where-Object { $_ -and $_ -ne '--force' })
# The binary resolves a relative CONDR_INSTALL_DIR against the process directory; use PowerShell's location.
if ($env:CONDR_INSTALL_DIR) {
  $env:CONDR_INSTALL_DIR = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($env:CONDR_INSTALL_DIR)
}

# Nothing else asked and the latest release already installed: nothing to do.
$installDir = if ($env:CONDR_INSTALL_DIR) { $env:CONDR_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Condr' }
$installed = Join-Path $installDir 'condr.exe'
if (-not $From -and -not $force -and -not $InstallArgs -and $Version -eq 'latest' -and $repo -eq 'condrdev/condr' -and (Test-Path -LiteralPath $installed -PathType Leaf)) {
  $latest = try { "$(Invoke-RestMethod -Uri 'https://condr.dev/version.txt')".Trim() } catch { '' }
  # Only a release prints nothing after its version; a nightly adds `(nightly)`.
  $current = try { $fields = "$(& $installed --version)".Trim().Split(' '); if ($fields.Count -eq 2) { $fields[1] } else { '' } } catch { '' }
  if ($latest -and $current.Split('+')[0] -eq $latest) {
    Write-Output "condr $current is already the latest release; pass --force to reinstall"
    return
  }
}

$stage = Join-Path ([IO.Path]::GetTempPath()) ("condr-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $stage | Out-Null
try {
  if (-not $From) {
    $api = if ($Version -eq 'latest') { "https://api.github.com/repos/$repo/releases/latest" }
           else { "https://api.github.com/repos/$repo/releases/tags/$Version" }
    $assets = (Invoke-RestMethod -Uri $api -Headers @{ Accept = 'application/vnd.github+json' }).assets
    $archiveAsset = $assets | Where-Object { $_.name -like 'condr-headless-*-windows-x86_64.zip' } | Select-Object -First 1
    $sumsAsset = $assets | Where-Object { $_.name -eq 'SHA256SUMS' } | Select-Object -First 1
    if (-not $archiveAsset -or -not $sumsAsset) { throw "no Windows headless build in release '$Version' of $repo" }
    $download = Join-Path $stage 'download'
    New-Item -ItemType Directory -Force $download | Out-Null
    $From = Join-Path $download $archiveAsset.name
    Write-Output "Downloading $($archiveAsset.name)"
    Invoke-WebRequest -Uri $archiveAsset.browser_download_url -OutFile $From
    Invoke-WebRequest -Uri $sumsAsset.browser_download_url -OutFile (Join-Path $download 'SHA256SUMS')
  }
  if (-not (Test-Path -LiteralPath $From -PathType Leaf)) { throw "archive not found: $From" }
  $From = (Resolve-Path -LiteralPath $From).Path
  $name = [IO.Path]::GetFileName($From)

  # A downloaded archive always has SHA256SUMS beside it; a -From archive may.
  $checksums = Join-Path (Split-Path -Parent $From) 'SHA256SUMS'
  if (Test-Path -LiteralPath $checksums) {
    $line = Get-Content $checksums | Where-Object { $_ -match ("\s" + [regex]::Escape($name) + "$") } | Select-Object -First 1
    if (-not $line) { throw "no checksum for $name" }
    $expected = ($line -split '\s+')[0].ToUpperInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $From).Hash.ToUpperInvariant()
    if ($actual -ne $expected) { throw "checksum mismatch: $name" }
  }

  $payload = Join-Path $stage 'payload'
  Expand-Archive -LiteralPath $From -DestinationPath $payload
  $condr = Join-Path $payload 'condr-headless\condr.exe'
  if (-not (Test-Path -LiteralPath $condr -PathType Leaf)) { throw "$name does not contain condr-headless\condr.exe" }
  & $condr server install @InstallArgs
  if ($LASTEXITCODE -ne 0) { throw "condr server install failed with exit code $LASTEXITCODE" }
} finally {
  if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
}
