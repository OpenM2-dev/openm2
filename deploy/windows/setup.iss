; openm2's own Windows setup (docs/specs/update.md, "openm2's own Windows
; release"): the launcher, the game's programs and launcher.toml, installed
; for the player alone and with no administrator, into a folder the launcher
; can update in place. release.yml compiles it with Inno Setup 6 from the
; zip's folder, then tries it: a silent install, its files, a silent
; uninstall. Its icon and pictures are the project's logo, from branding/
; (branding/README.md: not under the code's licence).
;
;   ISCC /DVersion=X.Y.Z /DSource=<the zip's folder> /O<out> deploy/windows/setup.iss

#ifndef Version
  #error Pass the version: /DVersion=X.Y.Z
#endif
#ifndef Source
  #error Pass the folder of the version's files and launcher.toml: /DSource=<folder>
#endif

; From the script's own folder.
#define Brand "..\..\branding"

[Setup]
; Never change it: Windows finds an install to upgrade or remove by it.
AppId={{34610A48-98A3-4B5D-91D6-639E1D6A4C5B}
AppName=openm2
AppVersion={#Version}
AppVerName=openm2 {#Version}
AppPublisher=openm2
AppPublisherURL=https://github.com/OpenM2-dev/openm2
AppSupportURL=https://github.com/OpenM2-dev/openm2/issues
AppUpdatesURL=https://github.com/OpenM2-dev/openm2/releases
; The player's own folder: the launcher updates itself and the game there,
; so it must be one they can write to, without an administrator.
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\openm2
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=openm2-setup
UninstallDisplayName=openm2
UninstallDisplayIcon={app}\openm2.ico
SetupIconFile={#Brand}\openm2.ico
WizardStyle=modern dark
WizardImageFile={#Brand}\setup-wizard.png
WizardSmallImageFile={#Brand}\setup-small.png
Compression=lzma2/max
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; The programs, the licences, the third-party notices, the README and
; launcher.toml: what the zip holds.
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
; The shortcuts' icon: the programs have none of their own.
Source: "{#Brand}\openm2.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\openm2"; Filename: "{app}\openm2-launcher.exe"; IconFilename: "{app}\openm2.ico"
Name: "{autodesktop}\openm2"; Filename: "{app}\openm2-launcher.exe"; IconFilename: "{app}\openm2.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\openm2-launcher.exe"; Description: "{cm:LaunchProgram,openm2}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; What the launcher makes beside the setup's files (docs/specs/update.md,
; "The install folder"): its versions, the player's import of their own game
; files and the saves' world made from them (the files themselves, wherever
; they are, are never touched), the player's choices and settings, the logs
; and the lock. The local saves (saves\) and the device's key
; (device-key.pem), which their characters belong to, stay: a new install
; finds them.
Type: filesandordirs; Name: "{app}\app"
Type: filesandordirs; Name: "{app}\previous"
Type: filesandordirs; Name: "{app}\staging"
Type: filesandordirs; Name: "{app}\imported"
Type: filesandordirs; Name: "{app}\importing"
Type: filesandordirs; Name: "{app}\content"
Type: filesandordirs; Name: "{app}\content-importing"
Type: files; Name: "{app}\client.toml"
Type: files; Name: "{app}\settings.toml"
Type: files; Name: "{app}\servers.toml"
Type: files; Name: "{app}\*.part"
Type: files; Name: "{app}\launcher.log"
Type: files; Name: "{app}\game.log"
Type: files; Name: "{app}\import.log"
Type: files; Name: "{app}\content.log"
Type: files; Name: "{app}\launcher.lock"
Type: files; Name: "{app}\openm2-launcher.old"
Type: dirifempty; Name: "{app}"
