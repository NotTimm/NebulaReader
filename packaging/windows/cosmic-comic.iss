; Inno Setup script for the Windows installer. Built by
; packaging/build-windows.ps1, which passes:
;   /DAppVersion=0.1.0-beta.3   the full version (shown to users)
;   /DNumericVersion=0.1.0      the X.Y.Z part (Windows file version)
;   /DStageDir=...              folder with cosmic-comic.exe and its DLLs
;   /DOutputDir=...             where the setup .exe goes
;
; Installs per user by default (no admin prompt) into
; %LOCALAPPDATA%\Programs\Cosmic Comic; the setup can still be run for all
; users from an elevated prompt. Supports winget's silent install
; (/VERYSILENT /SUPPRESSMSGBOXES /NORESTART, plus /CURRENTUSER or /ALLUSERS).

#define AppName "Cosmic Comic"
#define AppExe "cosmic-comic.exe"

[Setup]
; Never change AppId: it's how upgrades find the existing install.
AppId={{A6F0EC97-23D8-4BB5-A3F9-CE2247715EE7}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Tim Singel
AppPublisherURL=https://github.com/NotTimm/cosmic-reader
AppSupportURL=https://github.com/NotTimm/cosmic-reader/issues
AppUpdatesURL=https://github.com/NotTimm/cosmic-reader/releases
VersionInfoVersion={#NumericVersion}
VersionInfoProductVersion={#NumericVersion}
VersionInfoProductTextVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\res\icons\com.tsingel.CosmicComic.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
OutputDir={#OutputDir}
OutputBaseFilename=CosmicComic-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Close a running copy before upgrading files, and reopen it afterwards.
CloseApplications=yes
RestartApplications=yes
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Offer Cosmic Comic under "Open with" for comics and books without taking
; over the default app for those file types.
Root: HKA; Subkey: "Software\Classes\CosmicComic.Document"; ValueType: string; ValueName: ""; ValueData: "Comic or book"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\CosmicComic.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"
Root: HKA; Subkey: "Software\Classes\CosmicComic.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.cbz\OpenWithProgids"; ValueType: string; ValueName: "CosmicComic.Document"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.cbr\OpenWithProgids"; ValueType: string; ValueName: "CosmicComic.Document"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.epub\OpenWithProgids"; ValueType: string; ValueName: "CosmicComic.Document"; ValueData: ""; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
