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
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=crates\stonemite\assets\app.ico
PrivilegesRequired=lowest
DisableProgramGroupPage=yes

[Files]
Source: "target\release\stonemite.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "config\example.toml"; DestDir: "{app}"; DestName: "example.toml"; Flags: ignoreversion
Source: "THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Stonemite"; Filename: "{app}\stonemite.exe"
Name: "{userstartup}\Stonemite"; Filename: "{app}\stonemite.exe"; Tasks: autostart

[Tasks]
Name: "autostart"; Description: "Start Stonemite when Windows starts"; Flags: unchecked

[Run]
Filename: "{app}\stonemite.exe"; Description: "Launch Stonemite"; Flags: nowait postinstall skipifsilent

[Code]
const
  WebView2ClientKey = 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  WebView2BootstrapperUrl = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703';

function WebView2VersionAtRoot(RootKey: Integer): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(RootKey, WebView2ClientKey, 'pv', Version) and
    (Version <> '') and (CompareText(Version, '0.0.0.0') <> 0);
end;

function IsWebView2Installed(): Boolean;
begin
  Result := WebView2VersionAtRoot(HKCU) or WebView2VersionAtRoot(HKLM32);
  if IsWin64 and not Result then
    Result := WebView2VersionAtRoot(HKLM64);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  BootstrapperPath: String;
  ResultCode: Integer;
begin
  Result := '';
  if IsWebView2Installed() then
    Exit;

  try
    BootstrapperPath := ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe');
    ResultCode := -1;
    DownloadTemporaryFile(
      WebView2BootstrapperUrl,
      'MicrosoftEdgeWebview2Setup.exe',
      '',
      nil);
    if not Exec(BootstrapperPath, '/silent /install', '', SW_SHOW,
      ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
      Result := Format('Microsoft Edge WebView2 could not be installed (exit code %d). ' +
        'Stonemite was not installed. Check your internet connection and try again.', [ResultCode]);
  except
    Result := 'Microsoft Edge WebView2 could not be downloaded. ' +
      GetExceptionMessage + ' Check your internet connection and try again.';
  end;
end;

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
        // Strip surrounding TOML basic or literal string quotes.
        if (Length(Value) >= 2) and
           (((Value[1] = '"') and (Value[Length(Value)] = '"')) or
            ((Value[1] = '''') and (Value[Length(Value)] = ''''))) then
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
