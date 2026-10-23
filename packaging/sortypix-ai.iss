; Sortypix AI: a separate application installed beside Sortypix (own AppId, folder, menu entry).
; Built by build-installer-ai.ps1, which verifies the model files and passes their folder as
; /DModels and the generated notices as /DNotices. Never touches the Sortypix installation.
#define AppVersion "0.3.0"
#ifndef Models
  #error Build with build-installer-ai.ps1 (it verifies the models and defines Models).
#endif
#ifndef Notices
  #error Build with build-installer-ai.ps1 (it writes the notices and defines Notices).
#endif
[Setup]
AppId={{A6C7B8C5-53FC-4328-9F7A-47FD7275B225}
AppName=Sortypix AI
AppVersion={#AppVersion}
AppVerName=Sortypix AI {#AppVersion} beta
AppPublisher=Mickael Mancini
DefaultDirName={localappdata}\Programs\Sortypix AI
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
DisableWelcomePage=no
DisableReadyPage=yes
SetupIconFile=..\assets\sortypix.ico
UninstallDisplayIcon={app}\sortypix-ai.exe
UninstallDisplayName=Sortypix AI (beta)
Compression=lzma2
SolidCompression=yes
OutputDir=out
OutputBaseFilename=Sortypix-AI-Setup-{#AppVersion}
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
[Messages]
WelcomeLabel2=Sortypix AI (beta) is Sortypix with a model that runs on this computer and suggests which photos to set aside; it never moves a photo by itself and never goes online.%n%nIt needs about 0.6 GB of disk space and installs next to Sortypix without changing it (the two share their settings).
[Files]
Source: "..\target\release\sortypix-ai.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Notices}"; DestDir: "{app}"; DestName: "THIRD-PARTY-NOTICES.txt"; Flags: ignoreversion
Source: "..\assets\sortypix.png"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Models}\onnxruntime.dll"; DestDir: "{app}\models"; Flags: ignoreversion
Source: "{#Models}\visual-int8.onnx"; DestDir: "{app}\models"; Flags: ignoreversion
Source: "{#Models}\textual-int8.onnx"; DestDir: "{app}\models"; Flags: ignoreversion
Source: "{#Models}\tokenizer.json"; DestDir: "{app}\models"; Flags: ignoreversion
Source: "{#Models}\aesthetic-l14.bin"; DestDir: "{app}\models"; Flags: ignoreversion
[Icons]
Name: "{autoprograms}\Sortypix AI"; Filename: "{app}\sortypix-ai.exe"
Name: "{autodesktop}\Sortypix AI"; Filename: "{app}\sortypix-ai.exe"
[Registry]
Root: HKCU; Subkey: "Software\Classes\Directory\shell\SortypixAI"; ValueType: string; ValueName: ""; ValueData: "Open with Sortypix AI"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\SortypixAI"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\sortypix-ai.exe"
Root: HKCU; Subkey: "Software\Classes\Directory\shell\SortypixAI\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sortypix-ai.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\SortypixAI"; ValueType: string; ValueName: ""; ValueData: "Open with Sortypix AI"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\SortypixAI"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\sortypix-ai.exe"
Root: HKCU; Subkey: "Software\Classes\Directory\Background\shell\SortypixAI\command"; ValueType: string; ValueName: ""; ValueData: """{app}\sortypix-ai.exe"" ""%V"""
[Run]
Filename: "{app}\sortypix-ai.exe"; Description: "Launch Sortypix AI"; Flags: nowait postinstall skipifsilent
