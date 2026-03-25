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
Source: "target\release\dinput8.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "config\example.toml"; DestDir: "{app}"; DestName: "example.toml"; Flags: ignoreversion

[Icons]
Name: "{group}\Stonemite"; Filename: "{app}\stonemite.exe"
Name: "{userstartup}\Stonemite"; Filename: "{app}\stonemite.exe"; Tasks: autostart

[Tasks]
Name: "autostart"; Description: "Start Stonemite when Windows starts"; Flags: unchecked
Name: "notelemetry"; Description: "Disable anonymous usage telemetry"; Flags: unchecked

[Run]
Filename: "{app}\stonemite.exe"; Description: "Launch Stonemite"; Flags: nowait postinstall skipifsilent

[Code]
function GetEqDirFromConfig(): String;
var
  ConfigPath: String;
  Lines: TArrayOfString;
  I, P: Integer;
  Line, Value: String;
begin
  Result := 'C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest';
  ConfigPath := ExpandConstant('{userappdata}\Stonemite\config.toml');
  if not FileExists(ConfigPath) then
    Exit;
  if not LoadStringsFromFile(ConfigPath, Lines) then
    Exit;
  for I := 0 to GetArrayLength(Lines) - 1 do
  begin
    Line := Trim(Lines[I]);
    if Pos('eq_dir', Line) = 1 then
    begin
      P := Pos('=', Line);
      if P > 0 then
      begin
        Value := Trim(Copy(Line, P + 1, Length(Line)));
        // Strip surrounding quotes
        if (Length(Value) >= 2) and (Value[1] = '"') and (Value[Length(Value)] = '"') then
          Value := Copy(Value, 2, Length(Value) - 2);
        // Unescape backslashes
        StringChangeEx(Value, '\\', '\', True);
        if Value <> '' then
          Result := Value;
      end;
      Exit;
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ConfigDir: String;
  ConfigPath: String;
begin
  if CurStep = ssPostInstall then
  begin
    if IsTaskSelected('notelemetry') then
    begin
      ConfigDir := ExpandConstant('{userappdata}\Stonemite');
      ConfigPath := ConfigDir + '\config.toml';
      if not DirExists(ConfigDir) then
        ForceDirectories(ConfigDir);
      if not FileExists(ConfigPath) then
        SaveStringToFile(ConfigPath, 'telemetry = false' + #13#10, False)
      else
        SaveStringToFile(ConfigPath, #13#10 + 'telemetry = false' + #13#10, True);
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  EqDir: String;
  DllPath: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    EqDir := GetEqDirFromConfig();
    DllPath := EqDir + '\dinput8.dll';
    if FileExists(DllPath) then
      DeleteFile(DllPath);
  end;
end;
