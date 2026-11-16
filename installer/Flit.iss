#define Product "Flit Browser"
#define ProductVersion "0.5.0"
#ifndef BundleDir
  #error Build with Build-Setup.ps1 or pass --define=BundleDir=PATH to ISCC.
#endif

[Setup]
AppId={{7B05A21B-1FC7-4A43-B8C5-2C97474A79FA}
AppName={#Product}
AppVersion={#ProductVersion}
AppVerName={#Product} {#ProductVersion}
AppPublisher=Flit Browser
VersionInfoVersion={#ProductVersion}.0
VersionInfoDescription=Flit Browser Setup
DefaultDirName={localappdata}\Programs\Flit Browser
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
SetupArchitecture=x64
ArchitecturesAllowed=x64os
MinVersion=10.0.22000
UninstallDisplayIcon={app}\Flit.exe
SetupIconFile=..\assets\Flit.ico
OutputBaseFilename=FlitSetup-x64
Compression=lzma2/fast
SolidCompression=yes
WizardStyle=modern dynamic
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BundleDir}\*"; DestDir: "{app}"; Excludes: "debug.log"; Flags: ignoreversion recursesubdirs createallsubdirs

[INI]
Filename: "{app}\flit-installed.ini"; Section: "Flit"; Key: "Version"; String: "{#ProductVersion}"; Flags: uninsdeletesection

[UninstallDelete]
Type: files; Name: "{app}\flit-installed.ini"

[Icons]
Name: "{userprograms}\Flit Browser"; Filename: "{app}\Flit.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\Flit Browser"; Filename: "{app}\Flit.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\Flit.exe"; WorkingDir: "{app}"; Description: "{cm:LaunchProgram,{#Product}}"; Flags: nowait postinstall skipifsilent runasoriginaluser
