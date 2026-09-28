$ErrorActionPreference = "Stop"

. "$PSScriptRoot\ffmpeg-env.ps1"

$ProjectName = "JXZS"
$ProjectDir = Resolve-Path "$PSScriptRoot\.."
$OutputZip = Join-Path $ProjectDir "target\release\$ProjectName-$env:SCRCPY_MASK_OS.zip"
$BuildTarget = Join-Path $ProjectDir "target\release\scrcpy-mask.exe"
$AssetsDir = Join-Path $ProjectDir "assets"

& "$PSScriptRoot\prepare-adb.ps1"

Push-Location (Join-Path $ProjectDir "frontend")
pnpm build
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    exit $LASTEXITCODE
}
Pop-Location

Push-Location $ProjectDir
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    exit $LASTEXITCODE
}
Pop-Location

Write-Host "Build successful, creating zip package..."
$StageDir = Join-Path $ProjectDir ("target\release\JXZS-package-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $StageDir | Out-Null
Copy-Item -LiteralPath $BuildTarget -Destination (Join-Path $StageDir "JXZS.exe")
Copy-Item -LiteralPath $AssetsDir -Destination $StageDir -Recurse
Get-ChildItem -LiteralPath $StageDir -Recurse -File | Where-Object { $_.Name -match "scrcpy" } | ForEach-Object {
    $NewName = $_.Name -replace "scrcpy-mask", "JXZS" -replace "scrcpy", "JXZS"
    Rename-Item -LiteralPath $_.FullName -NewName $NewName
}
if (Get-ChildItem -LiteralPath $StageDir -Recurse | Where-Object { $_.Name -match "scrcpy" }) {
    throw "Package contains an unrenamed scrcpy path"
}
Compress-Archive -Path (Join-Path $StageDir "*") -DestinationPath $OutputZip -Force

Write-Host "Package created: $OutputZip"
