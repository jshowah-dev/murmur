; Build from the repo root: ISCC.exe /DAppVersion=X.Y.Z installer\murmur.iss
#ifndef AppVersion
  #error Pass the version: /DAppVersion=X.Y.Z
#endif

[Setup]
AppId={{3859BC9B-2892-4D3F-8616-9C7EB9B7AD57}
AppName=Murmur
AppVersion={#AppVersion}
AppVerName=Murmur {#AppVersion}
AppPublisher=Jeff Showah
AppPublisherURL=https://github.com/jshowah-dev/murmur
AppSupportURL=https://github.com/jshowah-dev/murmur/issues
; paths below are relative to the repo root
SourceDir=..
OutputDir=.
OutputBaseFilename=murmur-v{#AppVersion}-setup
DefaultDirName={localappdata}\Programs\Murmur
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=assets\murmur.ico
UninstallDisplayIcon={app}\murmur.exe
UninstallDisplayName=Murmur
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
CloseApplications=force
RestartApplications=no
; the autostart task's state comes from the registry, not the last install
UsePreviousTasks=no

[Tasks]
Name: "autostart"; Description: "Start Murmur with Windows"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "target\release\murmur.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\onnxruntime.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\onnxruntime_providers_shared.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\sherpa-onnx-c-api.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\sherpa-onnx-cxx-api.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Murmur"; Filename: "{app}\murmur.exe"
Name: "{autodesktop}\Murmur"; Filename: "{app}\murmur.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\murmur.exe"; Description: "{cm:LaunchProgram,Murmur}"; Flags: nowait postinstall skipifsilent

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';

var
  TasksPrimed: Boolean;

{ On an upgrade, show start-with-Windows as it is now (the tray may have changed it). }
procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpSelectTasks) and not TasksPrimed then
  begin
    TasksPrimed := True;
    if WizardForm.PrevAppDir <> '' then
    begin
      if RegValueExists(HKCU, RunKey, 'Murmur') then
        WizardSelectTasks('autostart')
      else
        WizardSelectTasks('!autostart');
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    if WizardIsTaskSelected('autostart') then
    begin
      RegWriteStringValue(HKCU, RunKey, 'Murmur', '"' + ExpandConstant('{app}\murmur.exe') + '"');
      RegDeleteValue(HKCU, ApprovedKey, 'Murmur');
    end
    else
      RegDeleteValue(HKCU, RunKey, 'Murmur');
  end;
end;

// Restart Manager closes Murmur for setup but not for uninstall, so stop the installed copy here.
// Only the one in the install folder: a zip copy or a dev build keeps running.
function InitializeUninstall(): Boolean;
var
  Exe: String;
  Code: Integer;
begin
  Exe := ExpandConstant('{app}\murmur.exe');
  StringChangeEx(Exe, '''', '''''', True);
  Exec('powershell.exe',
       '-NoProfile -NonInteractive -Command "$p = Get-Process murmur -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq ''' + Exe + ''' }; $p | Stop-Process -Force; $p | Wait-Process -Timeout 5"',
       '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    RegDeleteValue(HKCU, RunKey, 'Murmur');
    RegDeleteValue(HKCU, ApprovedKey, 'Murmur');
  end;
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
  begin
    if MsgBox('Delete the downloaded speech model (about 660 MB)? You''d need to download it again if you reinstall.',
              mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{localappdata}\Murmur\models'), True, True, True);
      RemoveDir(ExpandConstant('{localappdata}\Murmur'));
    end;
    if MsgBox('Delete your settings, dictionary and snippets?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(ExpandConstant('{userappdata}\Murmur'), True, True, True);
  end;
end;
