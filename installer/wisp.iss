; Inno Setup script for Wisp. Built by CI:
;   iscc /DAppVersion=0.1.0 /DSourceExe=..\target\release\wisp.exe installer\wisp.iss
; Installs per user (no admin prompt) into %LOCALAPPDATA%\Programs\Wisp.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\wisp.exe"
#endif

[Setup]
AppId={{6F1C2B7E-4A3D-4E8B-9C51-2D7A9E0B5F14}
AppName=Wisp
AppVersion={#AppVersion}
AppVerName=Wisp {#AppVersion}
AppPublisher=Wisp
AppPublisherURL=https://github.com/Xzpy909/Wisp
AppSupportURL=https://github.com/Xzpy909/Wisp/issues
AppUpdatesURL=https://github.com/Xzpy909/Wisp/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Wisp
DisableProgramGroupPage=yes
DisableDirPage=auto
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=Wisp-{#AppVersion}-setup
SetupIconFile=..\assets\icons\wisp.ico
UninstallDisplayIcon={app}\wisp.exe
UninstallDisplayName=Wisp
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; Wisp is closed with "wisp --quit" below, so Restart Manager is not needed.
CloseApplications=no

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Wisp"; Filename: "{app}\wisp.exe"; Comment: "Clipboard history"

[Registry]
; Wisp writes its own "Start with Windows" entry; remove it on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Wisp"; Flags: uninsdeletevalue dontcreatekey

[Run]
Filename: "{app}\wisp.exe"; Description: "Start Wisp"; Flags: nowait postinstall skipifsilent
; Silent updates restart Wisp in the tray.
Filename: "{app}\wisp.exe"; Parameters: "--background"; Flags: nowait; Check: WizardSilent

[UninstallRun]
Filename: "{app}\wisp.exe"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitWisp"

[Code]
// Close a running Wisp before its files are replaced.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Exe: String;
  Code: Integer;
begin
  Result := '';
  Exe := ExpandConstant('{app}\wisp.exe');
  if FileExists(Exe) then
    Exec(Exe, '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

// Offer to remove clip history and settings too.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Data := ExpandConstant('{userappdata}\Wisp');
    if DirExists(Data) and not UninstallSilent then
      if MsgBox('Also delete your clip history, clip sets and settings?' #13#10 + Data,
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(Data, True, True, True);
  end;
end;
