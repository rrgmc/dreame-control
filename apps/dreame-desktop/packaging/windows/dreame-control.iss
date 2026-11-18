; Inno Setup script for the Dreame Control installer. Run by package.ps1:
;   iscc /DAppVersion=0.2.0 /DSourceExe=<path to dreame-desktop.exe> /DOutputDir=<dir> dreame-control.iss
; It installs for the current user only, so it needs no administrator rights.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6C1C2E0A-5C0F-4B5E-9C7B-2E2B0D6F4A11}
AppName=Dreame Control
AppVersion={#AppVersion}
AppPublisher=Dreame Control
DefaultDirName={localappdata}\Programs\Dreame Control
DefaultGroupName=Dreame Control
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={#OutputDir}
OutputBaseFilename=DreameControl-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\Dreame Control.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Dreame Control.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Dreame Control"; Filename: "{app}\Dreame Control.exe"
Name: "{autodesktop}\Dreame Control"; Filename: "{app}\Dreame Control.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Dreame Control.exe"; Description: "Start Dreame Control"; Flags: nowait postinstall skipifsilent
