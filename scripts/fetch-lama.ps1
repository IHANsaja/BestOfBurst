# Downloads the LaMa inpainting model used by the watermark remover's AI edge clean-up
# (src/inpaint.rs) into %LOCALAPPDATA%\BestOfBurst\models\ and checks its SHA-256.
# The model is Apache-2.0, from https://huggingface.co/Carve/LaMa-ONNX (about 200 MB).
# Pass -Dest <folder> to put it elsewhere (for example a "models" folder beside bestofburst.exe).
param(
    [string]$Dest = (Join-Path $env:LOCALAPPDATA "BestOfBurst\models")
)
$ErrorActionPreference = "Stop"
$url = "https://huggingface.co/Carve/LaMa-ONNX/resolve/main/lama_fp32.onnx"
$sha256 = "1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6"
$file = Join-Path $Dest "lama_fp32.onnx"

New-Item -ItemType Directory -Force $Dest | Out-Null
if ((Test-Path $file) -and ((Get-FileHash $file -Algorithm SHA256).Hash.ToLower() -eq $sha256)) {
    Write-Host "Already in place: $file"
    exit 0
}
$part = "$file.part"
Write-Host "Downloading $url ..."
$ProgressPreference = "SilentlyContinue"   # the progress bar makes Invoke-WebRequest very slow
Invoke-WebRequest -Uri $url -OutFile $part -UseBasicParsing
$hash = (Get-FileHash $part -Algorithm SHA256).Hash.ToLower()
if ($hash -ne $sha256) {
    Remove-Item $part -Force
    throw "The download does not match: SHA-256 $hash, expected $sha256"
}
Move-Item $part $file -Force
Write-Host "Saved and checked: $file"
