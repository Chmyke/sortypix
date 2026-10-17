#define AppVersion "0.2.0"
[Setup]
AppId={{738617A0-A20C-4305-8CF2-7DB1491952AB}
AppName=Sortypix
AppVersion={#AppVersion}
AppPublisher=Mickael Mancini
DefaultDirName={localappdata}\Programs\Sortypix
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
DisableReadyPage=yes
SetupIconFile=..\assets\sortypix.ico
UninstallDisplayIcon={app}\sortypix.exe
Compression=lzma2
SolidCompression=yes
OutputDir=out
OutputBaseFilename=Sortypix-Setup-{#AppVersion}
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
[Files]
Source: "..\target\release\sortypix.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\sortypix_shell.dll"; DestDir: "{app}"; DestName: "sortypix_shell-{#AppVersion}.dll"; Flags: ignoreversion
Source: "..\assets\sortypix.png"; DestDir: "{app}"; Flags: ignoreversion
Source: "out\Sortypix-Explorer-{#AppVersion}.msix"; DestDir: "{app}"; Flags: ignoreversion
Source: "explorer\register.ps1"; DestDir: "{app}"; DestName: "register-explorer.ps1"; Flags: ignoreversion
[InstallDelete]
Type: files; Name: "{app}\Sortypix-Explorer-*.msix"
[Icons]
Name: "{autoprograms}\Sortypix"; Filename: "{app}\sortypix.exe"
Name: "{autodesktop}\Sortypix"; Filename: "{app}\sortypix.exe"
[Registry]
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Sortypix"; ValueType: string; ValueName: ""; ValueData: "Open with Sortypix"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Sortypix"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\sortypix.exe"
Root: HKCU; Subkey: "Software\Classes\Directory\shell\Sortypix\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sortypix.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Sortypix"; ValueType: string; ValueName: ""; ValueData: "Open with Sortypix"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Sortypix"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\sortypix.exe"
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\Sortypix\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sortypix.exe"" ""%V"""
[Code]
function InitializeUninstall(): Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\register-explorer.ps1') + '" -Unregister',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := Result and (ResultCode = 0);
  if not Result then begin
    Log('Uninstall stopped: Explorer identity could not be removed. Files retained.');
    if not UninstallSilent then
      MsgBox('Windows could not remove the Sortypix Explorer integration. No application files were removed. Close open context menus and try again. Details: %LOCALAPPDATA%\Sortypix\explorer-registration.txt', mbError, MB_OK);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  WindowsVersion: TWindowsVersion;
begin
  if CurStep = ssPostInstall then begin
    GetWindowsVersionEx(WindowsVersion);
    if WindowsVersion.Build >= 22000 then begin
      if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
        '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + ExpandConstant('{app}\register-explorer.ps1') + '"',
        '', SW_HIDE, ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then begin
        Log('Modern Explorer menu registration failed. See %LOCALAPPDATA%\Sortypix\explorer-registration.txt');
        if not WizardSilent then
          MsgBox('Sortypix is installed, but Windows could not register its modern Explorer menu. See %LOCALAPPDATA%\Sortypix\explorer-registration.txt for details.', mbError, MB_OK);
      end;
    end;
  end;
end;
[Run]
Filename: "{app}\sortypix.exe"; Description: "Launch Sortypix"; Flags: nowait postinstall skipifsilent
