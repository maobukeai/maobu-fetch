function Expand-AuditArchive([string]$SevenZip,[string]$Archive,[string]$Destination) {
    $destination=Assert-AuditTempPath $Destination
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    $listing=@(& $SevenZip l -slt $Archive 2>&1 | ForEach-Object {$_.ToString()})
    if($LASTEXITCODE -ne 0){throw "7-Zip could not list candidate: $([IO.Path]::GetFileName($Archive))"}
    $start=[Array]::IndexOf($listing,'----------')
    if($start -lt 0){throw 'Archive parser did not produce a file table'}
    $entries=@($listing[($start+1)..($listing.Length-1)] | Where-Object {$_ -match '^Path = '} | ForEach-Object {$_.Substring(7)})
    if($entries.Count -gt 5000){throw 'Candidate archive exceeds the bounded inventory size'}
    foreach($entry in $entries) {
        $normalized=$entry.Replace([char]92,[char]47)
        if($normalized.StartsWith('/') -or $normalized -match '^[a-zA-Z]:' -or ($normalized.Split('/') -contains '..')) {throw 'Unsafe archive entry path; extraction refused'}
    }
    $listing | Set-Content -LiteralPath ($destination+'.listing.txt') -Encoding utf8
    $messages=@(& $SevenZip x -y ('-o'+$destination) $Archive 2>&1 | ForEach-Object {$_.ToString()})
    $messages | Set-Content -LiteralPath ($destination+'.extraction.txt') -Encoding utf8
    if($LASTEXITCODE -ne 0){throw "7-Zip extraction failed: $([IO.Path]::GetFileName($Archive))"}
    return [pscustomobject]@{Entries=$entries;Files=@(Get-ChildItem -LiteralPath $destination -Recurse -File);ParserWarnings=@($listing | Where-Object {$_ -match 'BadCmd|ERROR|WARN'})}
}

function New-AuditExtensionZips([string]$Workspace,[string]$Output,[string]$Version) {
    $source=Join-Path $Workspace 'extension/src'
    $distribution=Join-Path $Workspace 'extension/dist'
    $manifest=Join-Path $distribution 'manifest.json'
    if(-not (Test-Path -LiteralPath $manifest)){throw 'Build the extension before inspecting release candidates'}
    $expected=@(Get-ChildItem -LiteralPath $source -Recurse -File | Where-Object {$_.Name -notlike '*.test.js'})
    foreach($file in $expected) {
        $relative=$file.FullName.Substring($source.Length).TrimStart([IO.Path]::DirectorySeparatorChar)
        $built=Join-Path (Join-Path $distribution 'src') $relative
        if(-not (Test-Path -LiteralPath $built) -or (Get-FileHash -LiteralPath $file.FullName).Hash -ne (Get-FileHash -LiteralPath $built).Hash){throw 'Extension distribution does not match its current source'}
    }
    $actual=@(Get-ChildItem -LiteralPath $distribution -Recurse -File)
    if($actual.Count -ne $expected.Count+1 -or @($actual | Where-Object {$_.Name -like '*.test.js'}).Count -gt 0){throw 'Extension distribution has unexpected or test files'}
    if((Get-FileHash -LiteralPath (Join-Path $Workspace 'extension/manifest.json')).Hash -ne (Get-FileHash -LiteralPath $manifest).Hash){throw 'Extension manifest is stale'}
    $metadata=Read-AuditJson $manifest
    if($metadata.version -ne $Version){throw 'Extension version does not match the candidate'}
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    Add-Type -AssemblyName System.IO.Compression
    $versioned=Assert-AuditTempPath (Join-Path $Output "maobu-fetch-extension-v$Version.zip")
    $legacy=Assert-AuditTempPath (Join-Path $Output 'extension.zip')
    [IO.Compression.ZipFile]::CreateFromDirectory($distribution,$versioned,[IO.Compression.CompressionLevel]::Optimal,$false)
    Copy-Item -LiteralPath $versioned -Destination $legacy -ErrorAction Stop
    return @((Get-Item -LiteralPath $versioned),(Get-Item -LiteralPath $legacy))
}
