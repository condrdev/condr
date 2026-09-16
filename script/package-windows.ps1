param(
  [string] $Version,
  [string] $Commit = $env:GITHUB_SHA,
  [string] $Dist = 'dist',
  [switch] $Release
)
$ErrorActionPreference = 'Stop'
if (-not $Version) {
  $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
  $Version = ($metadata.packages | Where-Object { $_.name -eq 'condr-server' } | Select-Object -First 1).version
}
if (-not $Version) { throw 'could not determine Condr version' }
if (-not $Commit) { throw '-Commit is required' }
$packageVersion = $Version
if (-not $Release) { $packageVersion += '-' + $Commit.Substring(0, 12) }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

$fullStage = Join-Path $Dist 'condr'
New-Item -ItemType Directory -Force -Path $fullStage | Out-Null
Copy-Item target\release\condr-gui.exe, target\release\condr.exe, LICENSE $fullStage
Set-Content -Encoding ascii -Path (Join-Path $fullStage BUILD-COMMIT) -Value $Commit
Compress-Archive -Path $fullStage -DestinationPath (Join-Path $Dist "condr-$packageVersion-windows-x86_64.zip") -Force

$headlessStage = Join-Path $Dist 'condr-headless'
New-Item -ItemType Directory -Force -Path $headlessStage | Out-Null
Copy-Item target\release\condr.exe, LICENSE $headlessStage
Set-Content -Encoding ascii -Path (Join-Path $headlessStage BUILD-COMMIT) -Value $Commit
Compress-Archive -Path $headlessStage -DestinationPath (Join-Path $Dist "condr-headless-$packageVersion-windows-x86_64.zip") -Force

$iscc = @(
  (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
  (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
  (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 is required' }
$bundleDir = (Resolve-Path -LiteralPath $fullStage).Path
& $iscc "/DBundleDir=$bundleDir" "/DAppVersion=$Version" "/DOutputBaseFilename=condr-$packageVersion-windows-x86_64" packaging\condr.iss
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE" }
