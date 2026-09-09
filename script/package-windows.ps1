param(
  [string] $Version = '0.1.0',
  [string] $Commit = $env:GITHUB_SHA,
  [string] $Dist = 'dist'
)
$ErrorActionPreference = 'Stop'
$shortSha = $Commit.Substring(0, 12)
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

$fullStage = Join-Path $Dist 'condr'
New-Item -ItemType Directory -Force -Path $fullStage | Out-Null
Copy-Item target\release\condr-gui.exe, target\release\condr.exe, LICENSE $fullStage
Set-Content -Encoding ascii -Path (Join-Path $fullStage BUILD-COMMIT) -Value $Commit
Compress-Archive -Path $fullStage -DestinationPath (Join-Path $Dist "condr-windows-x86_64-$Version-$shortSha.zip") -Force

$cliStage = Join-Path $Dist 'condr-cli'
New-Item -ItemType Directory -Force -Path $cliStage | Out-Null
Copy-Item target\release\condr.exe, LICENSE $cliStage
Set-Content -Encoding ascii -Path (Join-Path $cliStage BUILD-COMMIT) -Value $Commit
Compress-Archive -Path $cliStage -DestinationPath (Join-Path $Dist "condr-windows-x86_64-$Version-$shortSha-cli.zip") -Force

$iscc = @(
  (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
  (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 is required' }
& $iscc "/DAppVersion=$Version" "/DOutputBaseFilename=condr-windows-x86_64-$Version-$shortSha-setup" packaging\condr.iss
Copy-Item script\install-condr.ps1 (Join-Path $Dist install-condr.ps1) -Force
