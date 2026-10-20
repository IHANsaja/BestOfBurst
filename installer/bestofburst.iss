; Inno Setup script for the BestOfBurst Windows installers (64-bit and 32-bit).
; Easiest: run installer\build.ps1, which builds both and the portable exes.
; By hand, after `cargo build --release` (and `--target i686-pc-windows-msvc` for 32-bit):
;   ISCC.exe /DArch=64 installer\bestofburst.iss
;   ISCC.exe /DArch=32 installer\bestofburst.iss
; Output: installer\Output\BestOfBurst-<version>-Setup-<64|32>bit.exe

#define AppName "BestOfBurst"
#ifndef AppVersion
  #define AppVersion "0.2.1"
#endif
#ifndef Arch
  #define Arch "64"
#endif
#define AppExe "bestofburst.exe"
#if Arch == "64"
  #define ExeDir "..\target\release"
#else
  #define ExeDir "..\target\i686-pc-windows-msvc\release"
#endif

[Setup]
AppId={{6F3C2A1E-9B4D-4E7A-8C51-B0B5F1A7E2D4}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion} ({#Arch}-bit)
AppPublisher=Ihan Hansaja
AppPublisherURL=https://github.com/IHANsaja/BestOfBurst
AppSupportURL=https://github.com/IHANsaja/BestOfBurst/issues
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=Output
OutputBaseFilename=BestOfBurst-{#AppVersion}-Setup-{#Arch}bit
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
#if Arch == "64"
; The 64-bit build needs 64-bit Windows.
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#else
; The 32-bit build runs on 32-bit and 64-bit Windows.
ArchitecturesAllowed=x86compatible
#endif
; Installs per user without admin rights; the dialog offers an all-users install.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayIcon={app}\{#AppExe}
SetupIconFile=..\assets\logo\bestofburst.ico
WizardImageFile=wizard-large-100.bmp,wizard-large-200.bmp
WizardSmallImageFile=wizard-small-100.bmp,wizard-small-200.bmp
UninstallDisplayName={#AppName} ({#Arch}-bit)

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExeDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\assets\fonts\OFL-Archivo.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\assets\fonts\OFL-MartianMono.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
