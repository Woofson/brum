; ==============================================================================
; 🐕 Brum - Windows Native Setup Installer (Inno Setup Script)
; Author: Bolt J Woofson <bolt@boop.no>
; Repository: https://github.com/Woofson/brum
; ==============================================================================

#define MyAppName "Brum"
#ifndef MyAppVersion
  #define MyAppVersion "1.2.0"
#endif
#define MyAppPublisher "Bolt J Woofson @ Woofsons Lab"
#define MyAppURL "https://www.arf.ac"
#define MyAppExeName "Brum.exe"
#define MyCliExeName "brum-cli.exe"

[Setup]
AppId={{D37F8A1B-824E-4B02-990C-5B6A84F819FA}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} v{#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
LicenseFile=..\..\LICENSE
OutputDir=..\..\dist\windows
OutputBaseFilename=Brum-Setup-v{#MyAppVersion}
SetupIconFile=..\..\assets\brum.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
AlwaysShowComponentsList=yes
AlwaysShowDirOnReadyPage=yes
AlwaysShowGroupOnReadyPage=yes
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=commandline
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Types]
Name: "standalone"; Description: "Standalone Desktop Application (Single Computer)"
Name: "service"; Description: "Windows Background Service (Headless Autostart Daemon / Fleet Server)"
Name: "full"; Description: "Full Suite (Desktop Application + Background Service + Context Menu)"
Name: "custom"; Description: "Custom Installation Options"; Flags: iscustom

[Components]
Name: "standalone"; Description: "Standalone Desktop Application (Brum.exe - Desktop UI & Shortcuts)"; Types: standalone full custom; Flags: checkablealone
Name: "service"; Description: "Windows Background Service (Brum-cli.exe autostart system daemon)"; Types: service full custom; Flags: checkablealone
Name: "contextmenu"; Description: "Windows Explorer Context Menu Integration ('Open in Brum')"; Types: standalone service full custom

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Components: standalone; Flags: unchecked
Name: "startservice"; Description: "Start Brum Windows Service immediately after installation"; GroupDescription: "Service Options:"; Components: service

[Files]
; Core Binaries & Assets
Source: "..\..\dist\windows\brum\brum-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\dist\windows\brum\Brum.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\..\dist\windows\brum\WebView2Loader.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\..\dist\windows\brum\config.toml"; DestDir: "{app}"; Flags: ignoreversion onlyifdoesntexist
Source: "..\..\dist\windows\brum\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\dist\windows\brum\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\brum.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "register-context-menu.reg"; DestDir: "{app}"; Flags: ignoreversion; Components: contextmenu
Source: "unregister-context-menu.reg"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\brum.ico"; Components: standalone
Name: "{group}\Brum (Web Console)"; Filename: "http://127.0.0.1:4040"; IconFilename: "{app}\brum.ico"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\brum.ico"; Tasks: desktopicon; Components: standalone

[Registry]
Root: HKLM; Subkey: "Software\Classes\Directory\shell\Brum"; ValueType: string; ValueName: ""; ValueData: "Open in Brum"; Flags: uninsdeletekey; Components: contextmenu
Root: HKLM; Subkey: "Software\Classes\Directory\shell\Brum"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\brum.ico"""; Flags: uninsdeletekey; Components: contextmenu
Root: HKLM; Subkey: "Software\Classes\Directory\shell\Brum\command"; ValueType: string; ValueName: ""; ValueData: """{app}\brum-cli.exe"" --open ""%1"""; Flags: uninsdeletekey; Components: contextmenu
Root: HKLM; Subkey: "Software\Classes\Directory\Background\shell\Brum"; ValueType: string; ValueName: ""; ValueData: "Open in Brum"; Flags: uninsdeletekey; Components: contextmenu
Root: HKLM; Subkey: "Software\Classes\Directory\Background\shell\Brum"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\brum.ico"""; Flags: uninsdeletekey; Components: contextmenu
Root: HKLM; Subkey: "Software\Classes\Directory\Background\shell\Brum\command"; ValueType: string; ValueName: ""; ValueData: """{app}\brum-cli.exe"" --open ""%V"""; Flags: uninsdeletekey; Components: contextmenu

[Run]
; Install Windows Service if component selected
Filename: "{app}\{#MyCliExeName}"; Parameters: "service install"; StatusMsg: "Registering Brum Windows Service..."; Flags: runhidden waituntilterminated; Components: service
; Start Windows Service if task selected
Filename: "{app}\{#MyCliExeName}"; Parameters: "service start"; StatusMsg: "Starting Brum Windows Service..."; Flags: runhidden waituntilterminated; Tasks: startservice; Components: service
; Post-install launch for standalone desktop app
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent; Components: standalone

[UninstallRun]
; Stop and remove Windows Service cleanly if installed
Filename: "{app}\{#MyCliExeName}"; Parameters: "service stop"; Flags: runhidden waituntilterminated
Filename: "{app}\{#MyCliExeName}"; Parameters: "service uninstall"; Flags: runhidden waituntilterminated
