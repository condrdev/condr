#ifndef AppVersion
; Release builds pass /DAppVersion from the workspace package version.
#define AppVersion "0.0.0"
#endif
#ifndef OutputBaseFilename
#define OutputBaseFilename "condr-windows-x86_64-setup"
#endif

[Setup]
SourceDir=..
AppId={{9B6E4C3B-502D-4CBF-A2E3-39B5E9E0A1A8}
AppName=Condr
AppVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Condr
DefaultGroupName=Condr
PrivilegesRequired=lowest
ChangesEnvironment=yes
OutputDir=dist
OutputBaseFilename={#OutputBaseFilename}
UninstallDisplayIcon={app}\condr-gui.exe

[Types]
Name: "full"; Description: "GUI + CLI"
Name: "cli"; Description: "CLI only"

[Components]
Name: "full"; Description: "GUI + CLI"; Types: full
Name: "cli"; Description: "CLI only"; Types: full cli

[Files]
Source: "target\release\condr.exe"; DestDir: "{app}"; Components: full or cli
Source: "target\release\condr-gui.exe"; DestDir: "{app}"; Components: full

[Icons]
Name: "{group}\Condr"; Filename: "{app}\condr-gui.exe"; Components: full
Name: "{userdesktop}\Condr"; Filename: "{app}\condr-gui.exe"; Components: full; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Flags: preservestringtype

[Code]
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
