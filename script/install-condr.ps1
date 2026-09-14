param(
  [string] $From,
  [string] $Version = 'nightly',
  [switch] $Yes
)
# Install only the headless CLI/server; preserve any existing GUI files.
$ErrorActionPreference = 'Stop'

$root = $env:CONDR_INSTALL_DIR
if (-not $root) { $root = Join-Path $env:LOCALAPPDATA 'Programs\Condr' }
$root = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($root)
$stage = Join-Path ([IO.Path]::GetTempPath()) ("condr-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $stage | Out-Null
$pending = $null
try {
  $downloaded = -not $From
  if (-not $From) {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw 'pass -From ARCHIVE or install GitHub CLI (gh)' }
    $repo = $env:CONDR_REPO
    if (-not $repo) { $repo = 'condrdev/condr' }
    gh release download $Version --repo $repo --dir $stage --pattern 'condr-cli-*-windows-x86_64.zip' --pattern SHA256SUMS --clobber
    if ($LASTEXITCODE -ne 0) { throw 'CLI download failed' }
    $archives = @(Get-ChildItem $stage -Filter 'condr-cli-*.zip')
    if ($archives.Count -ne 1) { throw 'expected one Windows CLI archive' }
    $From = $archives[0].FullName
  }
  if (-not (Test-Path -LiteralPath $From -PathType Leaf)) { throw "archive not found: $From" }
  $From = (Resolve-Path -LiteralPath $From).Path
  $checksums = Join-Path (Split-Path -Parent $From) 'SHA256SUMS'
  if ($downloaded -and -not (Test-Path $checksums)) { throw 'SHA256SUMS is missing' }
  if (Test-Path $checksums) {
    $name = [IO.Path]::GetFileName($From)
    $line = Get-Content $checksums | Where-Object { $_ -match ("\s" + [regex]::Escape($name) + "$") } | Select-Object -First 1
    if (-not $line) { throw "no checksum for $name" }
    $expected = ($line -split '\s+')[0].ToUpperInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $From).Hash.ToUpperInvariant()
    if ($actual -ne $expected) { throw "checksum mismatch: $name" }
  }
  $extracted = Join-Path $stage 'extracted'
  Expand-Archive -LiteralPath $From -DestinationPath $extracted
  $payload = Join-Path $extracted 'condr-cli'
  if (-not (Test-Path (Join-Path $payload 'condr.exe')) -or
      (Get-ChildItem $extracted -Recurse -Filter 'condr-gui.exe')) {
    throw 'expected a CLI-only archive containing condr-cli\condr.exe'
  }
  foreach ($name in 'condr.exe', 'LICENSE', 'BUILD-COMMIT') {
    if (-not (Test-Path -LiteralPath (Join-Path $payload $name) -PathType Leaf)) { throw "CLI archive is missing $name" }
  }
  $binary = Join-Path $root 'condr.exe'
  if ((Test-Path -LiteralPath $binary) -and -not $Yes) {
    $answer = Read-Host "Condr is already installed in $root. Replace the CLI (keep other files)? [y/N]"
    if ($answer -notin 'y', 'yes') { Write-Output 'Installation cancelled.'; return }
  }
  New-Item -ItemType Directory -Force $root | Out-Null
  $pending = Join-Path $root ('condr-' + [guid]::NewGuid() + '.new')
  Copy-Item -LiteralPath (Join-Path $payload 'condr.exe') -Destination $pending
  if (Test-Path -LiteralPath $binary) {
    [IO.File]::Replace($pending, $binary, [System.Management.Automation.Language.NullString]::Value)
  } else {
    [IO.File]::Move($pending, $binary)
  }
  foreach ($name in 'LICENSE', 'BUILD-COMMIT') {
    Copy-Item -LiteralPath (Join-Path $payload $name) -Destination $root -Force
  }
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $parts = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $root })
  [Environment]::SetEnvironmentVariable('Path', (($parts + $root) -join ';'), 'User')
  Write-Output "Condr installed in $root. Open a new terminal to use condr."
} finally {
  if ($pending -and (Test-Path -LiteralPath $pending)) { Remove-Item -LiteralPath $pending -Force }
  if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
}
