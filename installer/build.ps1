# Builds every release file into installer\Output:
#   BestOfBurst-<version>-Setup-64bit.exe     BestOfBurst-<version>-Portable-64bit.exe
#   BestOfBurst-<version>-Setup-32bit.exe     BestOfBurst-<version>-Portable-32bit.exe
# Needs Rust (with the i686-pc-windows-msvc target) and Inno Setup 6.
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found" }

cargo build --release
cargo build --release --target i686-pc-windows-msvc

$out = Join-Path $PSScriptRoot "Output"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item target\release\bestofburst.exe (Join-Path $out "BestOfBurst-$version-Portable-64bit.exe") -Force
Copy-Item target\i686-pc-windows-msvc\release\bestofburst.exe (Join-Path $out "BestOfBurst-$version-Portable-32bit.exe") -Force
& $iscc "/DAppVersion=$version" "/DArch=64" (Join-Path $PSScriptRoot "bestofburst.iss")
& $iscc "/DAppVersion=$version" "/DArch=32" (Join-Path $PSScriptRoot "bestofburst.iss")
Get-ChildItem $out -Filter "BestOfBurst-$version-*" | Select-Object Name, Length
