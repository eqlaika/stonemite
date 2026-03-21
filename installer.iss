[Setup]
AppName=Stonemite
AppVerName=Stonemite
AppVersion={#AppVersion}
AppPublisher=Laikasoft
AppPublisherURL=https://github.com/eqlaika/stonemite
DefaultDirName={autopf}\Stonemite
DefaultGroupName=Stonemite
UninstallDisplayIcon={app}\stonemite.exe
OutputDir=dist
OutputBaseFilename=stonemite-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=app\assets\app.ico
PrivilegesRequired=lowest
DisableProgramGroupPage=yes

[Files]
Source: "target\release\stonemite.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "config\example.toml"; DestDir: "{app}"; DestName: "example.toml"; Flags: ignoreversion

[Icons]
Name: "{group}\Stonemite"; Filename: "{app}\stonemite.exe"
Name: "{userstartup}\Stonemite"; Filename: "{app}\stonemite.exe"; Tasks: autostart

[Tasks]
Name: "autostart"; Description: "Start Stonemite when Windows starts"; Flags: unchecked

[Run]
Filename: "{app}\stonemite.exe"; Description: "Launch Stonemite"; Flags: nowait postinstall skipifsilent
