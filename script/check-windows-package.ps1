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
  $guiZip = @(Get-ChildItem $Dist -Filter 'condr-*-windows-x86_64.zip' | Where-Object Name -NotLike 'condr-cli-*')
  $cliZip = @(Get-ChildItem $Dist -Filter 'condr-cli-*-windows-x86_64.zip')
  $installer = @(Get-ChildItem $Dist -Filter 'condr-*-windows-x86_64.exe')
  if ($guiZip.Count -ne 1 -or $cliZip.Count -ne 1 -or $installer.Count -ne 1) { throw 'expected one GUI ZIP, CLI ZIP and installer' }
  Expand-Archive $guiZip[0].FullName (Join-Path $stage 'bundle')
  $bundle = Join-Path $stage 'bundle\condr'
  foreach ($name in 'condr.exe', 'condr-gui.exe', 'LICENSE', 'BUILD-COMMIT') {
    if (-not (Test-Path (Join-Path $bundle $name))) { throw "GUI ZIP is missing $name" }
  }
  $commit = (Get-Content (Join-Path $bundle 'BUILD-COMMIT') -Raw).Trim()
  if ($env:GITHUB_SHA -and $commit -ne $env:GITHUB_SHA) { throw 'GUI ZIP has the wrong commit' }

  # Exercise the shell's icon extraction, including the installer executable.
  Add-Type -AssemblyName System.Drawing
  $expectedIcon = [Drawing.Icon]::new((Join-Path $PSScriptRoot '../packaging/icons/condr.ico'), 32, 32)
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

  # Install GUI + CLI, without creating shortcuts on the test runner.
  $process = Start-Process -FilePath $installer[0].FullName -Wait -PassThru -ArgumentList @(
    '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/NOICONS', '/TYPE=full',
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

  # Empty input must cancel an update, even when a GUI shares this directory.
  $env:CONDR_INSTALL_DIR = $gui
  Set-Content (Join-Path $gui 'condr.exe') 'previous-cli'
  $previousHash = (Get-FileHash (Join-Path $gui 'condr.exe')).Hash
  '' | powershell.exe -NoProfile -File (Join-Path $PSScriptRoot 'install-condr.ps1') -From $cliZip[0].FullName
  if ($LASTEXITCODE -ne 0 -or (Get-FileHash (Join-Path $gui 'condr.exe')).Hash -ne $previousHash) { throw 'cancelled installation changed the CLI' }
  'yes' | powershell.exe -NoProfile -File (Join-Path $PSScriptRoot 'install-condr.ps1') -From $cliZip[0].FullName
  if ($LASTEXITCODE -ne 0 -or (Get-FileHash (Join-Path $gui 'condr.exe')).Hash -eq $previousHash) { throw 'confirmed installation did not update the CLI' }

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
      & (Join-Path $PSScriptRoot 'install-condr.ps1') -From $cliZip[0].FullName -Yes
      & (Join-Path $destination 'condr.exe') server --help
      if ($LASTEXITCODE -ne 0) { throw 'installed CLI failed' }
      if ((Get-Content (Join-Path $destination 'BUILD-COMMIT') -Raw).Trim() -ne $commit) { throw 'CLI ZIP has the wrong commit' }
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
  try { & (Join-Path $PSScriptRoot 'install-condr.ps1') -From $guiZip[0].FullName -Yes }
  catch { $rejected = $true }
  if (-not $rejected) { throw 'CLI script accepted a GUI ZIP' }
  Write-Output 'Windows package content, native installation and CLI update checks passed.'
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
