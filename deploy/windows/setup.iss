; openm2's own Windows setup (docs/specs/update.md, "openm2's own Windows
; release"): the launcher, the game's programs and launcher.toml, installed
; for the player alone and with no administrator, into a folder the launcher
; can update in place. release.yml compiles it with Inno Setup 6 from the
; zip's folder, then tries it: a silent install, its files, a silent
; uninstall.
;
;   ISCC /DVersion=X.Y.Z /DSource=<the zip's folder> /O<out> deploy/windows/setup.iss

#ifndef Version
  #error Pass the version: /DVersion=X.Y.Z
#endif
#ifndef Source
  #error Pass the folder of the version's files and launcher.toml: /DSource=<folder>
#endif

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
UninstallDisplayIcon={app}\openm2-launcher.exe
WizardStyle=modern
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

[Icons]
Name: "{autoprograms}\openm2"; Filename: "{app}\openm2-launcher.exe"
Name: "{autodesktop}\openm2"; Filename: "{app}\openm2-launcher.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\openm2-launcher.exe"; Description: "{cm:LaunchProgram,openm2}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; What the launcher makes beside the setup's files (docs/specs/update.md,
; "The install folder"): its versions, the player's import of their own game
; files (the files themselves, wherever they are, are never touched), the
; player's choices and settings, the logs and the lock.
Type: filesandordirs; Name: "{app}\app"
Type: filesandordirs; Name: "{app}\previous"
Type: filesandordirs; Name: "{app}\staging"
Type: filesandordirs; Name: "{app}\imported"
Type: filesandordirs; Name: "{app}\importing"
Type: files; Name: "{app}\client.toml"
Type: files; Name: "{app}\settings.toml"
Type: files; Name: "{app}\servers.toml"
Type: files; Name: "{app}\*.part"
Type: files; Name: "{app}\launcher.log"
Type: files; Name: "{app}\game.log"
Type: files; Name: "{app}\import.log"
Type: files; Name: "{app}\launcher.lock"
Type: files; Name: "{app}\openm2-launcher.old"
Type: dirifempty; Name: "{app}"
