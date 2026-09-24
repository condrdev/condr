# Run on a disposable Windows user/CI runner: exercises the per-user installer.
param([string] $Dist = 'dist')
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Native package checks require Windows' }
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{9B6E4C3B-502D-4CBF-A2E3-39B5E9E0A1A8}_is1'
if (Test-Path -LiteralPath $uninstallKey) { throw 'Run this check under a test user without an installed Condr GUI' }
$Dist = (Resolve-Path $Dist).Path
$stage = Join-Path ([IO.Path]::GetTempPath()) ('condr-package-check-' + [guid]::NewGuid())
$originalPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$originalInstallDir = $env:CONDR_INSTALL_DIR
$gui = Join-Path $stage 'gui'
New-Item -ItemType Directory -Path $stage | Out-Null

try {
  $guiZip = @(Get-ChildItem $Dist -Filter 'condr-*-windows-x86_64.zip' | Where-Object Name -NotLike 'condr-headless-*')
  $headlessZip = @(Get-ChildItem $Dist -Filter 'condr-headless-*-windows-x86_64.zip')
  $installer = @(Get-ChildItem $Dist -Filter 'condr-*-windows-x86_64.exe')
  if ($guiZip.Count -ne 1 -or $headlessZip.Count -ne 1 -or $installer.Count -ne 1) { throw 'expected one desktop ZIP, headless ZIP and installer' }
  Expand-Archive $guiZip[0].FullName (Join-Path $stage 'bundle')
  $bundle = Join-Path $stage 'bundle\condr'
  foreach ($name in 'condr.exe', 'condr-gui.exe', 'LICENSE', 'BUILD-COMMIT') {
    if (-not (Test-Path (Join-Path $bundle $name))) { throw "GUI ZIP is missing $name" }
  }
  $commit = (Get-Content (Join-Path $bundle 'BUILD-COMMIT') -Raw).Trim()
  if ($env:GITHUB_SHA -and $commit -ne $env:GITHUB_SHA) { throw 'GUI ZIP has the wrong commit' }

  # Exercise the shell's icon extraction, including the installer executable.
  Add-Type -AssemblyName System.Drawing
  $expectedIcon = [Drawing.Icon]::new((Join-Path $PSScriptRoot '../assets/brand/condr.ico'), 32, 32)
  $expectedBitmap = $expectedIcon.ToBitmap()
  try {
    foreach ($executable in (Join-Path $bundle 'condr.exe'), (Join-Path $bundle 'condr-gui.exe'), $installer[0].FullName) {
      $icon = [Drawing.Icon]::ExtractAssociatedIcon($executable)
      $bitmap = $icon.ToBitmap()
      try {
        if ($bitmap.Size -ne $expectedBitmap.Size) { throw "wrong icon size in $executable" }
        for ($y = 0; $y -lt $bitmap.Height; $y++) {
          for ($x = 0; $x -lt $bitmap.Width; $x++) {
            if ($bitmap.GetPixel($x, $y) -ne $expectedBitmap.GetPixel($x, $y)) { throw "wrong application icon in $executable" }
          }
        }
      } finally { $bitmap.Dispose(); $icon.Dispose() }
    }
  } finally { $expectedBitmap.Dispose(); $expectedIcon.Dispose() }

  # Install the desktop build, without creating shortcuts on the test runner.
  $process = Start-Process -FilePath $installer[0].FullName -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOICONS',
    "/DIR=`"$gui`"", "/LOG=`"$(Join-Path $stage 'installer.log')`""
  )
  if ($process.ExitCode -ne 0) {
    Get-Content (Join-Path $stage 'installer.log')
    throw "GUI installer failed: $($process.ExitCode)"
  }
  if ((Get-Content (Join-Path $gui 'BUILD-COMMIT') -Raw).Trim() -ne $commit) { throw 'GUI installer has the wrong commit' }
  if (-not (Test-Path (Join-Path $gui 'LICENSE') -PathType Leaf)) { throw 'GUI installer is missing LICENSE' }
  $guiHash = (Get-FileHash (Join-Path $gui 'condr-gui.exe')).Hash
  & (Join-Path $gui 'condr.exe') server --help
  if ($LASTEXITCODE -ne 0) { throw 'installed GUI bundle CLI failed' }
  if ($gui -notin ([Environment]::GetEnvironmentVariable('Path', 'User') -split ';')) { throw 'GUI installer did not register PATH' }

  # An upgrade over the same directory must not add a second PATH entry.
  $process = Start-Process -FilePath $installer[0].FullName -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOICONS', "/DIR=`"$gui`""
  )
  if ($process.ExitCode -ne 0) { throw "GUI installer upgrade failed: $($process.ExitCode)" }
  $guiPathEntries = @([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_ -eq $gui })
  if ($guiPathEntries.Count -ne 1) { throw 'GUI installer must register PATH exactly once' }

  # The headless ZIP carries only the CLI, at this commit.
  Expand-Archive $headlessZip[0].FullName (Join-Path $stage 'headless')
  $headless = Join-Path $stage 'headless\condr-headless'
  foreach ($name in 'condr.exe', 'LICENSE', 'BUILD-COMMIT') {
    if (-not (Test-Path (Join-Path $headless $name) -PathType Leaf)) { throw "headless ZIP is missing $name" }
  }
  if (Test-Path (Join-Path $headless 'condr-gui.exe')) { throw 'headless ZIP contains the GUI' }
  if ((Get-Content (Join-Path $headless 'BUILD-COMMIT') -Raw).Trim() -ne $commit) { throw 'headless ZIP has the wrong commit' }
  # The binary knows its own build: `condr <version>+<12-char commit>` (ADR 0027),
  # followed by `(nightly)` exactly when this is not a release.
  $versionLine = (& (Join-Path $headless 'condr.exe') --version | Out-String).Trim()
  if ($versionLine.Split(' ')[1].Split('+')[-1] -ne $commit.Substring(0, 12)) { throw "condr --version says '$versionLine', not commit $commit" }
  if ($versionLine.EndsWith(' (nightly)') -ne ($env:CONDR_RELEASE -ne '1')) { throw "condr --version says '$versionLine' with CONDR_RELEASE=$env:CONDR_RELEASE" }

  # Updating replaces the CLI without asking and leaves the GUI alone (ADR 0016).
  $env:CONDR_INSTALL_DIR = $gui
  Set-Content (Join-Path $gui 'condr.exe') 'previous-cli'
  $previousHash = (Get-FileHash (Join-Path $gui 'condr.exe')).Hash
  & (Join-Path $PSScriptRoot 'install-condr.ps1') -From $headlessZip[0].FullName
  if ((Get-FileHash (Join-Path $gui 'condr.exe')).Hash -eq $previousHash) { throw 'update did not replace the CLI' }

  # Resolve a relative CLI directory against PowerShell's location, even when
  # it differs from the process directory; also update an absolute GUI path.
  $currentDirectory = Join-Path $stage 'current'
  New-Item -ItemType Directory -Path $currentDirectory | Out-Null
  $originalProcessDirectory = [Environment]::CurrentDirectory
  Push-Location $currentDirectory
  try {
    [Environment]::CurrentDirectory = $stage
    foreach ($destination in (Join-Path $currentDirectory 'cli'), $gui) {
      $env:CONDR_INSTALL_DIR = if ($destination -eq $gui) { $gui } else { 'cli' }
      & (Join-Path $PSScriptRoot 'install-condr.ps1') -From $headlessZip[0].FullName
      & (Join-Path $destination 'condr.exe') server --help
      if ($LASTEXITCODE -ne 0) { throw 'installed CLI failed' }
      $pathEntries = @([Environment]::GetEnvironmentVariable('Path', 'User') -split ';' | Where-Object { $_ -eq $destination })
      if ($pathEntries.Count -ne 1) { throw 'CLI installer must register PATH exactly once' }
    }
  } finally {
    Pop-Location
    [Environment]::CurrentDirectory = $originalProcessDirectory
  }
  if (Test-Path (Join-Path $currentDirectory 'cli\condr-gui.exe')) { throw 'CLI script installed a GUI' }
  if ((Get-FileHash (Join-Path $gui 'condr-gui.exe')).Hash -ne $guiHash) { throw 'CLI update changed the GUI' }

  $rejected = $false
  try { & (Join-Path $PSScriptRoot 'install-condr.ps1') -From $guiZip[0].FullName }
  catch { $rejected = $true }
  if (-not $rejected) { throw 'headless script accepted a desktop ZIP' }
  Write-Output 'Windows package content, native installation and headless update checks passed.'
} finally {
  try {
    $uninstaller = Join-Path $gui 'unins000.exe'
    if (Test-Path $uninstaller) {
      $process = Start-Process $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
      if ($process.ExitCode -ne 0) { throw "test uninstall failed: $($process.ExitCode)" }
    }
  } finally {
    [Environment]::SetEnvironmentVariable('Path', $originalPath, 'User')
    $env:CONDR_INSTALL_DIR = $originalInstallDir
    Remove-Item $stage -Recurse -Force
  }
}
