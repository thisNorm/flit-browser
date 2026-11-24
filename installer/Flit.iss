#define Product "Flit Browser"
#define ProductVersion "0.41.0"
#define BrowserClient "FlitBrowser"
#define BrowserKey "Software\Clients\StartMenuInternet\" + BrowserClient
#define CapabilitiesKey BrowserKey + "\Capabilities"
#define UrlProgId "FlitBrowser.URL"
#define UrlProgIdKey "Software\Classes\" + UrlProgId
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
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BundleDir}\*"; DestDir: "{app}"; Excludes: "debug.log"; Flags: ignoreversion recursesubdirs createallsubdirs

[Registry]
; Advertise HTTP(S) support without writing any default association.
Root: HKCU; Subkey: "{#UrlProgIdKey}"; ValueType: string; ValueData: "Flit Browser URL"; Flags: uninsdeletekey
Root: HKCU; Subkey: "{#UrlProgIdKey}"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "{#UrlProgIdKey}\DefaultIcon"; ValueType: string; ValueData: """{app}\Flit.exe"",0"
Root: HKCU; Subkey: "{#UrlProgIdKey}\shell\open\command"; ValueType: string; ValueData: """{app}\Flit.exe"" -- ""%1"""

Root: HKCU; Subkey: "{#BrowserKey}"; ValueType: string; ValueData: "{#Product}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "{#BrowserKey}\DefaultIcon"; ValueType: string; ValueData: """{app}\Flit.exe"",0"
Root: HKCU; Subkey: "{#BrowserKey}\shell\open\command"; ValueType: string; ValueData: """{app}\Flit.exe"""
Root: HKCU; Subkey: "{#CapabilitiesKey}"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#Product}"
Root: HKCU; Subkey: "{#CapabilitiesKey}"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Open HTTP and HTTPS links with Flit Browser."
Root: HKCU; Subkey: "{#CapabilitiesKey}"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: """{app}\Flit.exe"",0"
Root: HKCU; Subkey: "{#CapabilitiesKey}\Startmenu"; ValueType: string; ValueName: "StartMenuInternet"; ValueData: "{#BrowserClient}"
Root: HKCU; Subkey: "{#CapabilitiesKey}\URLAssociations"; ValueType: string; ValueName: "http"; ValueData: "{#UrlProgId}"
Root: HKCU; Subkey: "{#CapabilitiesKey}\URLAssociations"; ValueType: string; ValueName: "https"; ValueData: "{#UrlProgId}"
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#Product}"; ValueData: "{#CapabilitiesKey}"; Flags: uninsdeletevalue

[INI]
Filename: "{app}\flit-installed.ini"; Section: "Flit"; Key: "Version"; String: "{#ProductVersion}"; Flags: uninsdeletesection

[UninstallDelete]
Type: files; Name: "{app}\flit-installed.ini"

[Icons]
Name: "{userprograms}\Flit Browser"; Filename: "{app}\Flit.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\Flit Browser"; Filename: "{app}\Flit.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\Flit.exe"; WorkingDir: "{app}"; Description: "{cm:LaunchProgram,{#Product}}"; Flags: nowait postinstall skipifsilent runasoriginaluser
