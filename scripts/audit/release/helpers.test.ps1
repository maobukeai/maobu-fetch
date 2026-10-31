$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'files.ps1')
. (Join-Path $PSScriptRoot 'archive.ps1')
$temporary=Join-Path ([IO.Path]::GetTempPath()) ('maobu-release-inspector-test-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporary | Out-Null
$source=Join-Path $temporary 'synthetic.bin'
$prefix='C:\Synthetic Developer\project'
[IO.File]::WriteAllBytes($source,[Text.Encoding]::UTF8.GetBytes($prefix))
if(@(Get-AuditPathHits @($source) @($prefix)).Count -eq 0){throw 'UTF-8 path scanner missed the fixture'}
[IO.File]::WriteAllBytes($source,[byte[]](@(0x7f)+[Text.Encoding]::Unicode.GetBytes($prefix)))
if(@(Get-AuditPathHits @($source) @($prefix)).Count -eq 0){throw 'Unaligned UTF-16 path scanner missed the fixture'}
[IO.File]::WriteAllBytes($source,[Text.Encoding]::UTF8.GetBytes($prefix.Replace([char]92,[char]47)))
if(@(Get-AuditPathHits @($source) @($prefix)).Count -eq 0){throw 'Forward-slash path scanner missed the fixture'}
$failed=$false
try{Copy-AuditArtifact $source (Join-Path $temporary 'must-not-copy.bin') ([datetime]::UtcNow.AddHours(1)) | Out-Null}catch{$failed=$true}
if(-not $failed -or (Test-Path -LiteralPath (Join-Path $temporary 'must-not-copy.bin'))){throw 'Old-build freshness guard did not fail before copying'}
if(-not (Test-AuditMediaName 'nested/ffmpeg.exe') -or -not (Test-AuditMediaName 'FFPROBE.EXE') -or (Test-AuditMediaName 'maobu-fetch.exe')){throw 'Media executable file-name guard failed'}
Add-Type -AssemblyName System.IO.Compression.FileSystem
Add-Type -AssemblyName System.IO.Compression
$archive=Join-Path $temporary 'synthetic-unsafe.zip'
$stream=[IO.File]::Create($archive)
$zip=[IO.Compression.ZipArchive]::new($stream,[IO.Compression.ZipArchiveMode]::Create)
try{
    $entry=$zip.CreateEntry('../must-not-write.txt').Open()
    try{$bytes=[Text.Encoding]::UTF8.GetBytes('synthetic');$entry.Write($bytes,0,$bytes.Length)}finally{$entry.Dispose()}
}finally{$zip.Dispose();$stream.Dispose()}
$failed=$false
try{Expand-AuditArchive (Get-Command 7z.exe -ErrorAction Stop).Source $archive (Join-Path $temporary 'extract') | Out-Null}catch{$failed=$true}
if(-not $failed -or (Test-Path -LiteralPath (Join-Path $temporary 'must-not-write.txt'))){throw 'Unsafe archive path was not refused before extraction'}
$jsonPath=Join-Path $temporary 'utf8-without-bom.json'
$title=([string][char]0x732b)+[char]0x6b65
[IO.File]::WriteAllText($jsonPath,('{"title":"'+$title+'"}'),[Text.UTF8Encoding]::new($false))
if((Read-AuditJson $jsonPath).title -ne $title){throw 'UTF-8 JSON without BOM was decoded using the system codepage'}
$portable=Join-Path $temporary 'synthetic-portable.bin'
$packaged=Join-Path $temporary 'synthetic-packaged.bin'
$content='synthetic|__TAURI_BUNDLE_TYPE_VAR_UNK|tail'
[IO.File]::WriteAllBytes($portable,[Text.Encoding]::ASCII.GetBytes($content))
foreach($kind in @('msi','nsis')){
    $tag=if($kind -eq 'msi'){'MSI'}else{'NSS'}
    [IO.File]::WriteAllBytes($packaged,[Text.Encoding]::ASCII.GetBytes($content.Replace('_UNK',('_'+$tag))))
    if(-not (Compare-AuditApplication $portable $packaged $kind).Matches){throw 'Exact Tauri package-type-only change was rejected'}
}
[IO.File]::WriteAllBytes($packaged,[Text.Encoding]::ASCII.GetBytes($content.Replace('_UNK','_MSI').Replace('|tail','|fail')))
if((Compare-AuditApplication $portable $packaged 'msi').Matches){throw 'Same-length one-byte tampering outside the marker was accepted'}
[IO.File]::WriteAllBytes($packaged,[Text.Encoding]::ASCII.GetBytes($content.Replace('_UNK','_NSS')))
if((Compare-AuditApplication $portable $packaged 'msi').Matches){throw 'Wrong package marker was accepted'}
[IO.File]::WriteAllBytes($portable,[Text.Encoding]::ASCII.GetBytes($content+$content))
[IO.File]::WriteAllBytes($packaged,[Text.Encoding]::ASCII.GetBytes(($content+$content).Replace('_UNK','_MSI')))
if((Compare-AuditApplication $portable $packaged 'msi').Matches){throw 'Duplicate complete UNK markers were accepted'}
[pscustomobject]@{Kind='SYNTHETIC_HELPER_TEST_ONLY';Passed=12;Output=$temporary} | ConvertTo-Json
