param(
  [string] $From,
  [switch] $CliOnly,
  [string] $Version = 'dev'
)
$ErrorActionPreference = 'Stop'

$root = Join-Path $env:LOCALAPPDATA 'Programs\Condr'
$stage = Join-Path ([IO.Path]::GetTempPath()) ("condr-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $stage | Out-Null
try {
  if (-not $From) {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw 'pass -From ARCHIVE or install GitHub CLI (gh)' }
    $repo = $env:CONDR_REPO
    if (-not $repo) { $repo = 'condrdev/condr' }
    $pattern = if ($CliOnly) { 'condr-cli-*-windows-x86_64.zip' } else { 'condr-[0-9]*-windows-x86_64.zip' }
    gh release download $Version --repo $repo --dir $stage --pattern $pattern --pattern SHA256SUMS --clobber
    $archives = Get-ChildItem $stage -Filter '*.zip'
    $selected = $archives | Select-Object -First 1
    if (-not $selected) { throw "no matching Windows archive for $(if ($CliOnly) { 'CLI-only' } else { 'GUI + CLI' })" }
    $From = $selected.FullName
  }
  if (-not (Test-Path -LiteralPath $From -PathType Leaf)) { throw "archive not found: $From" }
  $checksums = Join-Path (Split-Path -Parent $From) 'SHA256SUMS'
  if (Test-Path $checksums) {
    $name = [IO.Path]::GetFileName($From)
    $line = Get-Content $checksums | Where-Object { $_ -match ("\s" + [regex]::Escape($name) + "$") } | Select-Object -First 1
    if (-not $line) { throw "no checksum for $name" }
    $expected = ($line -split '\s+')[0].ToUpperInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $From).Hash.ToUpperInvariant()
    if ($actual -ne $expected) { throw "checksum mismatch: $name" }
  }
  Expand-Archive -LiteralPath $From -DestinationPath $stage -Force
  $payload = Get-ChildItem $stage -Recurse -File | Where-Object { $_.Name -eq 'condr.exe' } | Select-Object -First 1
  if (-not $payload) { throw 'archive does not contain condr.exe' }
  $new = Join-Path $stage 'payload'
  New-Item -ItemType Directory -Force $new | Out-Null
  Copy-Item (Join-Path $payload.DirectoryName '*') $new -Recurse -Force
  if (-not $CliOnly) {
    $gui = Join-Path $payload.DirectoryName 'condr-gui.exe'
    if (-not (Test-Path $gui)) { throw 'GUI bundle does not contain condr-gui.exe' }
  }
  New-Item -ItemType Directory -Force (Split-Path $root) | Out-Null
  $old = $null
  try {
    if (Test-Path $root) {
      $old = $root + '.old-' + [guid]::NewGuid()
      Rename-Item $root $old
    }
    Rename-Item $new $root
    if ($old) { Remove-Item $old -Recurse -Force }
  } catch {
    if ($old -and -not (Test-Path $root) -and (Test-Path $old)) { Rename-Item $old $root }
    throw
  }
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $parts = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $root })
  [Environment]::SetEnvironmentVariable('Path', (($parts + $root) -join ';'), 'User')
  Write-Output "Condr installed in $root. Open a new terminal to use condr."
} finally {
  if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
}
