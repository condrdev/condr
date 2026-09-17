#ifndef AppVersion
; Release builds pass /DAppVersion from the workspace package version.
#define AppVersion "0.0.0"
#endif
#ifndef OutputBaseFilename
#define OutputBaseFilename "condr-" + AppVersion + "-windows-x86_64"
#endif
#ifndef BundleDir
#error "Build with script/package-windows.ps1 to supply BundleDir"
#endif

; Desktop build only: the GUI and the CLI/Server are always installed together.
; Headless machines use script/install-condr.ps1 with the condr-headless ZIP instead.
[Setup]
SourceDir=..
AppId={{9B6E4C3B-502D-4CBF-A2E3-39B5E9E0A1A8}
AppName=Condr
AppVersion={#AppVersion}
AppPublisher=Condr
AppPublisherURL=https://github.com/condrdev/condr
AppSupportURL=https://github.com/condrdev/condr/issues
AppUpdatesURL=https://github.com/condrdev/condr/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Condr
DefaultGroupName=Condr
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
; ConPTY needs Windows 10 1809.
MinVersion=10.0.17763
ArchitecturesAllowed=x64compatible
ChangesEnvironment=yes
CloseApplications=yes
WizardStyle=modern
OutputDir=dist
OutputBaseFilename={#OutputBaseFilename}
SetupIconFile=assets\brand\condr.ico
UninstallDisplayIcon={app}\condr-gui.exe

[Files]
Source: "{#BundleDir}\condr.exe"; DestDir: "{app}"
Source: "{#BundleDir}\condr-gui.exe"; DestDir: "{app}"
Source: "{#BundleDir}\LICENSE"; DestDir: "{app}"
Source: "{#BundleDir}\BUILD-COMMIT"; DestDir: "{app}"

[Icons]
Name: "{group}\Condr"; Filename: "{app}\condr-gui.exe"; AppUserModelID: "dev.condr.gui"
Name: "{userdesktop}\Condr"; Filename: "{app}\condr-gui.exe"; AppUserModelID: "dev.condr.gui"; Tasks: desktopicon

[Tasks]
Name: "addtopath"; Description: "Add the condr command to PATH"
Name: "desktopicon"; Description: "Create a desktop shortcut"

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Flags: preservestringtype; Tasks: addtopath; Check: NeedsAddPath

[Run]
Filename: "{app}\condr-gui.exe"; Description: "Launch Condr"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\condr.exe"; Parameters: "server stop"; Flags: runhidden; RunOnceId: "StopServer"

[Code]
function NeedsAddPath: Boolean;
var
  UserPath: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', UserPath) then
    UserPath := '';
  Result := Pos(';' + Uppercase(ExpandConstant('{app}')) + ';', ';' + Uppercase(UserPath) + ';') = 0;
end;

// The Server outlives the GUI, so an upgrade must stop it before replacing
// condr.exe; otherwise Restart Manager would kill it without a snapshot.
function PrepareToInstall(var NeedsRestart: Boolean): string;
var
  Cli: string;
  ResultCode: Integer;
  Attempt: Integer;
begin
  Result := '';
  Cli := ExpandConstant('{app}\condr.exe');
  if not FileExists(Cli) then
    exit;
  if not Exec(Cli, 'server stop', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
    exit;
  // stop returns once the Server acknowledges; wait for the process to go away.
  for Attempt := 1 to 50 do
  begin
    if not Exec(Cli, 'server status', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
      break;
    Sleep(200);
  end;
  Sleep(500);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  UserPath: string;
  InstallPath: string;
begin
  if CurUninstallStep <> usUninstall then
    exit;
  InstallPath := ExpandConstant('{app}');
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', UserPath) then
    exit;
  StringChangeEx(UserPath, ';' + InstallPath, '', True);
  StringChangeEx(UserPath, InstallPath + ';', '', True);
  if CompareText(UserPath, InstallPath) = 0 then
    UserPath := '';
  RegWriteStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', UserPath);
end;
