$ErrorActionPreference = "Stop"
$workspace = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$target = Join-Path $workspace "src-tauri\resources\tools"
$temp = Join-Path $workspace ".lumaget-tools-temp"
if (-not $temp.StartsWith($workspace, [StringComparison]::OrdinalIgnoreCase)) { throw "Temporary path escaped workspace" }
if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
New-Item -ItemType Directory -Path $temp, $target -Force | Out-Null

$ytUrl = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.06.09/yt-dlp.exe"
$ytHash = "3a48cb955d55c8821b60ccbdbbc6f61bc958f2f3d3b7ad5eaf3d83a543293a27"
$ffUrl = "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip"
$ffHash = "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec"
$ffmpegHash = "1326dde4c84ff1f96fe6b8916c5bed29e163e9b5dccf995f6f3db069d143ec5e"
$ffprobeHash = "b49ccc7c6547b141ad5a2f6ec69cc04323d7133d7704d70b331b904c63eecb07"

function Test-Hash([string]$Path, [string]$Sha256) {
  (Test-Path -LiteralPath $Path) -and ((Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() -eq $Sha256)
}
if ((Test-Hash (Join-Path $target "yt-dlp.exe") $ytHash) -and
    (Test-Hash (Join-Path $target "ffmpeg.exe") $ffmpegHash) -and
    (Test-Hash (Join-Path $target "ffprobe.exe") $ffprobeHash)) {
  if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
  Write-Host "Pinned media tools are already verified"
  exit 0
}

function Get-VerifiedFile([string]$Url, [string]$Path, [string]$Sha256) {
  Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Path
  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
  if ($actual -ne $Sha256) { throw "Checksum mismatch: $Path expected $Sha256 actual $actual" }
}

Get-VerifiedFile $ytUrl (Join-Path $target "yt-dlp.exe") $ytHash
$archive = Join-Path $temp "ffmpeg.zip"
Get-VerifiedFile $ffUrl $archive $ffHash
Expand-Archive -LiteralPath $archive -DestinationPath $temp -Force
$ffmpeg = Get-ChildItem -Path $temp -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
$ffprobe = Get-ChildItem -Path $temp -Recurse -Filter "ffprobe.exe" | Select-Object -First 1
if (-not $ffmpeg -or -not $ffprobe) { throw "Invalid FFmpeg archive layout" }
Copy-Item -LiteralPath $ffmpeg.FullName -Destination (Join-Path $target "ffmpeg.exe") -Force
Copy-Item -LiteralPath $ffprobe.FullName -Destination (Join-Path $target "ffprobe.exe") -Force
@{
  yt_dlp = @{ version = "2026.06.09"; sha256 = $ytHash; source = $ytUrl }
  ffmpeg = @{ version = "8.1.2-essentials"; archive_sha256 = $ffHash; source = $ffUrl }
} | ConvertTo-Json -Depth 3 | Set-Content -Encoding UTF8 (Join-Path $target "versions.json")
Remove-Item -LiteralPath $temp -Recurse -Force
Write-Host "Pinned media tools verified at $target"
