#ifndef PayloadDirectory
  #error PayloadDirectory is required
#endif
#ifndef ReleaseVersion
  #error ReleaseVersion is required
#endif
#ifndef OutputDirectory
  #error OutputDirectory is required
#endif

[Setup]
AppId=Yu.Editor.GitHub
AppName=Yu
AppVersion={#ReleaseVersion}
AppPublisher=Yu contributors
AppPublisherURL=https://github.com/xiaodou997/yu
AppSupportURL=https://github.com/xiaodou997/yu/issues
AppUpdatesURL=https://github.com/xiaodou997/yu/releases
DefaultDirName={localappdata}\Programs\Yu
DefaultGroupName=Yu
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir={#OutputDirectory}
OutputBaseFilename=Yu-{#ReleaseVersion}-windows-x64-setup
SetupIconFile=..\AppBundle\Yu.ico
UninstallDisplayIcon={app}\Yu.exe
UninstallDisplayName=Yu
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
SignedUninstaller=no
DisableProgramGroupPage=no
CloseApplications=no
RestartApplications=no
SetupLogging=yes
LicenseFile={#PayloadDirectory}\Licenses\Yu-Apache-2.0.txt

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
#if FileExists(AddBackslash(CompilerPath) + "Languages\ChineseSimplified.isl")
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
Name: "chinesetraditional"; MessagesFile: "compiler:Languages\ChineseTraditional.isl"
#endif
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PayloadDirectory}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Yu"; Filename: "{app}\Yu.exe"; WorkingDir: "{app}"
Name: "{group}\{cm:UninstallProgram,Yu}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Yu"; Filename: "{app}\Yu.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\Yu.exe"; Description: "{cm:LaunchProgram,Yu}"; Flags: nowait postinstall skipifsilent
