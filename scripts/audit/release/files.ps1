function Assert-AuditTempPath([string]$Path) {
    $resolved = [IO.Path]::GetFullPath($Path)
    $temporary = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar)
    if (-not $resolved.StartsWith($temporary + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw 'Audit output must stay inside TEMP' }
    return $resolved
}

function Add-AuditCheck($Checks,[string]$Name,[bool]$Passed,[string]$Detail) {
    [void]$Checks.Add([pscustomobject]@{Name=$Name;Passed=$Passed;Detail=$Detail})
}

function Read-AuditJson([string]$Path) {
    return Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Copy-AuditArtifact([string]$Source,[string]$Target,[datetime]$BuiltAfterUtc) {
    $file = Get-Item -LiteralPath $Source -ErrorAction Stop
    if (-not $file.PSIsContainer -and $file.LastWriteTimeUtc -lt $BuiltAfterUtc.ToUniversalTime()) { throw "Candidate predates this build: $($file.Name)" }
    $target = Assert-AuditTempPath $Target
    New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($target)) -Force | Out-Null
    Copy-Item -LiteralPath $file.FullName -Destination $target -ErrorAction Stop
    return Get-Item -LiteralPath $target
}

function Get-AuditArtifact($File,[string]$Kind) {
    [pscustomobject]@{Kind=$Kind;Name=$File.Name;Path=$File.FullName;Bytes=$File.Length;Sha256=(Get-FileHash -LiteralPath $File.FullName -Algorithm SHA256).Hash.ToLowerInvariant();LastWriteTimeUtc=$File.LastWriteTimeUtc.ToString('o')}
}

function Get-AuditVersion([string]$Path) {
    $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
    return [pscustomobject]@{ProductName=$info.ProductName;ProductVersion=$info.ProductVersion;FileVersion=$info.FileVersion;OriginalFilename=$info.OriginalFilename;Description=$info.FileDescription}
}

function Compare-AuditApplication([string]$Portable,[string]$Packaged,[ValidateSet('msi','nsis')][string]$Kind) {
    $original=[IO.File]::ReadAllBytes($Portable)
    $packed=[IO.File]::ReadAllBytes($Packaged)
    $originalHash=(Get-FileHash -LiteralPath $Portable -Algorithm SHA256).Hash.ToLowerInvariant()
    $packedHash=(Get-FileHash -LiteralPath $Packaged -Algorithm SHA256).Hash.ToLowerInvariant()
    $result=[ordered]@{Kind=$Kind;Matches=$false;PortableRawSha256=$originalHash;PackagedRawSha256=$packedHash;MarkerOffset=$null;NormalizedPackagedSha256=$null;Reason=$null}
    if($original.Length -ne $packed.Length){$result.Reason='Executable lengths differ';return [pscustomobject]$result}
    # tauri-utils 2.8.3 src/platform.rs:344-361 declares this exact mutable build marker.
    $prefix='__TAURI_BUNDLE_TYPE_VAR_'
    $marker=$prefix+'UNK'
    $text=[Text.Encoding]::ASCII.GetString($original)
    $index=$text.IndexOf($marker,[StringComparison]::Ordinal)
    if($index -lt 0 -or $text.LastIndexOf($marker,[StringComparison]::Ordinal) -ne $index){$result.Reason='Portable must contain one complete Tauri UNK bundle marker';return [pscustomobject]$result}
    $expected=$prefix+$(if($Kind -eq 'msi'){'MSI'}else{'NSS'})
    if([Text.Encoding]::ASCII.GetString($packed,$index,$marker.Length) -ne $expected){$result.Reason='Package bundle marker is not the expected type at the same offset';return [pscustomobject]$result}
    $result.MarkerOffset=$index+$prefix.Length
    $replacement=[Text.Encoding]::ASCII.GetBytes('UNK')
    [Array]::Copy($replacement,0,$packed,$result.MarkerOffset,3)
    $digest=[Security.Cryptography.SHA256]::Create()
    try{$normalized=([BitConverter]::ToString($digest.ComputeHash($packed))).Replace('-','').ToLowerInvariant()}finally{$digest.Dispose()}
    $result.NormalizedPackagedSha256=$normalized
    $result.Matches=$normalized -eq $originalHash
    $result.Reason=if($result.Matches){'Only the exact three-byte Tauri bundle type differs'}else{'Other executable bytes differ'}
    return [pscustomobject]$result
}

function Get-AuditPathHits([string[]]$Files,[string[]]$Prefixes) {
    $needles = @($Prefixes | ForEach-Object { $_; $_.Replace([char]92,[char]47) } | Sort-Object -Unique)
    $hits = foreach($file in $Files) {
        $bytes = [IO.File]::ReadAllBytes($file)
        $texts = @([pscustomobject]@{Encoding='UTF-8';Text=[Text.Encoding]::UTF8.GetString($bytes)},[pscustomobject]@{Encoding='UTF-16LE-even';Text=[Text.Encoding]::Unicode.GetString($bytes)})
        if($bytes.Length -gt 1){$texts += [pscustomobject]@{Encoding='UTF-16LE-odd';Text=[Text.Encoding]::Unicode.GetString($bytes,1,$bytes.Length-1)}}
        foreach($text in $texts) {
            foreach($needle in $needles) {
                if($needle -and $text.Text.IndexOf($needle,[StringComparison]::OrdinalIgnoreCase) -ge 0) {
                    [pscustomobject]@{File=$file;Encoding=$text.Encoding;Prefix=$needle}
                }
            }
        }
    }
    return @($hits)
}

function Test-AuditMediaName([string]$Name) {
    return [IO.Path]::GetFileName($Name.Replace('/',[IO.Path]::DirectorySeparatorChar)) -match '^(?i:yt-dlp|ffmpeg|ffprobe)\.exe$'
}
