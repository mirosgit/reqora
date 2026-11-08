; Reqora installer for Windows (Inno Setup 6.3+). Built by packaging\build-windows.ps1, or directly:
;   iscc /DAppVersion=0.2.0 packaging\windows\reqora.iss   ->  dist\Reqora-Setup-0.2.0-x64.exe
; Paths are relative to this file.

#ifndef AppVersion
  #error Pass the version: iscc /DAppVersion=x.y.z reqora.iss
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\reqora.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
; Identifies the app for upgrades and uninstall: never change it.
AppId={{0F2A3B5A-C836-45D2-892E-2C74940ACAE0}
AppName=Reqora
AppVersion={#AppVersion}
AppVerName=Reqora {#AppVersion}
AppPublisher=Reqora contributors
AppPublisherURL=https://github.com/mirosgit/reqora
AppSupportURL=https://github.com/mirosgit/reqora/issues
; Installs for the current user (no administrator rights needed) into
; %LOCALAPPDATA%\Programs\Reqora; the first page lets the user pick "all users" instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\Reqora
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\assets\reqora.ico
UninstallDisplayIcon={app}\reqora.exe
UninstallDisplayName=Reqora
VersionInfoVersion={#AppVersion}
VersionInfoProductName=Reqora
VersionInfoDescription=Reqora installer
OutputDir={#OutputDir}
OutputBaseFilename=Reqora-Setup-{#AppVersion}-x64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; An upgrade closes a running Reqora first (it saves its state on exit).
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "reqora.exe"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Reqora"; Filename: "{app}\reqora.exe"
Name: "{autodesktop}\Reqora"; Filename: "{app}\reqora.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\reqora.exe"; Description: "{cm:LaunchProgram,Reqora}"; Flags: nowait postinstall skipifsilent

; Collections, history and settings live in %APPDATA%\Reqora and are kept on uninstall.
