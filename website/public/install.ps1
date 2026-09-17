# Install the Condr headless build (CLI/Server) on Windows x86_64.
#   irm https://condr.dev/install.ps1 | iex
#   $env:CONDR_VERSION = 'v0.1.0'; irm https://condr.dev/install.ps1 | iex
#   $env:CONDR_INSTALL_ARGS = '--start'; irm https://condr.dev/install.ps1 | iex
# Downloads the archive from GitHub Releases, verifies SHA256SUMS,
# then runs `condr server install` with $env:CONDR_INSTALL_ARGS.
$ErrorActionPreference = 'Stop'

$repo = if ($env:CONDR_REPO) { $env:CONDR_REPO } else { 'condrdev/condr' }
$version = if ($env:CONDR_VERSION) { $env:CONDR_VERSION } else { 'nightly' }
$api = if ($version -eq 'latest') { "https://api.github.com/repos/$repo/releases/latest" }
       else { "https://api.github.com/repos/$repo/releases/tags/$version" }

$assets = (Invoke-RestMethod -Uri $api -Headers @{ Accept = 'application/vnd.github+json' }).assets
$archiveAsset = $assets | Where-Object { $_.name -like 'condr-headless-*-windows-x86_64.zip' } | Select-Object -First 1
$sumsAsset = $assets | Where-Object { $_.name -eq 'SHA256SUMS' } | Select-Object -First 1
if (-not $archiveAsset -or -not $sumsAsset) { throw "no Windows headless build in release '$version' of $repo" }

$stage = Join-Path ([IO.Path]::GetTempPath()) ("condr-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $stage | Out-Null
try {
  $archive = Join-Path $stage $archiveAsset.name
  Write-Output "Downloading $($archiveAsset.name)"
  Invoke-WebRequest -Uri $archiveAsset.browser_download_url -OutFile $archive
  $sums = Join-Path $stage 'SHA256SUMS'
  Invoke-WebRequest -Uri $sumsAsset.browser_download_url -OutFile $sums

  $line = Get-Content $sums | Where-Object { $_ -match ("\s" + [regex]::Escape($archiveAsset.name) + "$") } | Select-Object -First 1
  if (-not $line) { throw "no checksum for $($archiveAsset.name)" }
  $expected = ($line -split '\s+')[0].ToUpperInvariant()
  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToUpperInvariant()
  if ($actual -ne $expected) { throw "checksum mismatch: $($archiveAsset.name)" }

  Expand-Archive -LiteralPath $archive -DestinationPath $stage
  $condr = Join-Path $stage 'condr-headless\condr.exe'
  if (-not (Test-Path -LiteralPath $condr -PathType Leaf)) { throw 'archive does not contain condr-headless\condr.exe' }
  $installArgs = @()
  if ($env:CONDR_INSTALL_ARGS) { $installArgs = $env:CONDR_INSTALL_ARGS -split '\s+' }
  & $condr server install @installArgs
  if ($LASTEXITCODE -ne 0) { throw "condr server install failed with exit code $LASTEXITCODE" }
} finally {
  if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
}
