$ErrorActionPreference = 'Stop'
foreach ($tool in @('ffmpeg','ffprobe')) { if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw ('This opt-in probe requires an already installed '+$tool+'; no tool will be downloaded') } }
$buildOutput = & powershell -NoProfile -File (Join-Path $PSScriptRoot 'run_muxer_probe.ps1') -Cli
if ($LASTEXITCODE -ne 0) { throw 'Native muxer CLI compilation failed' }
$executable = ($buildOutput | Where-Object { $_ -like 'Probe executable:*' } | Select-Object -Last 1).Substring('Probe executable: '.Length)
$fixtureDirectory = Join-Path ([IO.Path]::GetTempPath()) ('maobu-muxer-real-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
function Read-PacketDigest([string]$Path,[string]$Selector) {
    $packets = & ffprobe -v error -select_streams $Selector -show_packets -show_data_hash sha256 -show_entries 'packet=pts,dts,duration,size,data_hash' -of json $Path
    if ($LASTEXITCODE -ne 0) { throw 'Packet hash inspection failed' }
    return (($packets -join "`n" | ConvertFrom-Json).packets | ConvertTo-Json -Depth 8 -Compress)
}
foreach ($mode in @('relative','absolute')) {
    $video = Join-Path $fixtureDirectory ($mode+'-video.mp4')
    $audio = Join-Path $fixtureDirectory ($mode+'-audio.mp4')
    $output = Join-Path $fixtureDirectory ($mode+'-merged.mp4')
    $flags = if ($mode -eq 'relative') { '+empty_moov+frag_keyframe+default_base_moof' } else { '+empty_moov+frag_keyframe' }
    & ffmpeg -y -hide_banner -loglevel error -f lavfi -i 'testsrc2=size=96x64:rate=10' -t 2 -an -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 5 -movflags $flags $video
    if ($LASTEXITCODE -ne 0) { throw 'Synthetic H.264 fixture generation failed' }
    & ffmpeg -y -hide_banner -loglevel error -f lavfi -i 'sine=frequency=440:sample_rate=48000' -t 2 -vn -c:a aac -b:a 64k -movflags $flags -frag_duration 500000 $audio
    if ($LASTEXITCODE -ne 0) { throw 'Synthetic AAC fixture generation failed' }
    & $executable $video $audio $output
    if ($LASTEXITCODE -ne 0) { throw ('Native '+$mode+' mux failed') }
    if ((Read-PacketDigest $video 'v:0') -cne (Read-PacketDigest $output 'v:0')) { throw 'Video payload SHA-256 or timestamps changed' }
    if ((Read-PacketDigest $audio 'a:0') -cne (Read-PacketDigest $output 'a:0')) { throw 'Audio payload SHA-256 or timestamps changed' }
    & ffmpeg -hide_banner -loglevel error -xerror -i $output -map 0:v:0 -map 0:a:0 -f null NUL
    if ($LASTEXITCODE -ne 0) { throw 'Merged audio/video full decoding failed' }
    $externalMux = Join-Path $fixtureDirectory ($mode+'-external-multitraf.mp4')
    & ffmpeg -y -hide_banner -loglevel error -i $video -i $audio -map 0:v:0 -map 1:a:0 -c copy -movflags $flags -frag_duration 500000 $externalMux
    if ($LASTEXITCODE -ne 0) { throw 'External multi-track fMP4 generation failed' }
    & $executable --validate $externalMux
    if ($LASTEXITCODE -ne 0) { throw 'Real multi-traf fMP4 validation failed' }
    Write-Output ($mode+': packet SHA-256/timestamps unchanged; full audio/video decode passed')
}
Write-Output ('Synthetic fixtures only: '+$fixtureDirectory)
