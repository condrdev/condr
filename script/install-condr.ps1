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
    gh release download $Version --repo $repo --dir $stage --pattern 'condr-windows-x86_64-*.zip' --clobber
    $From = (Get-ChildItem $stage -Filter '*.zip' | Select-Object -First 1).FullName
  }
  if (-not (Test-Path -LiteralPath $From -PathType Leaf)) { throw "archive not found: $From" }
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
