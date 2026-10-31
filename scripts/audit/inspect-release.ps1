[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$ReleaseDirectory,
    [Parameter(Mandatory=$true)][datetime]$BuildStartedUtc,
    [string]$OutputDirectory,
    [string]$SevenZip
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'release/files.ps1')
. (Join-Path $PSScriptRoot 'release/msi.ps1')
. (Join-Path $PSScriptRoot 'release/archive.ps1')

$workspace=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$release=(Resolve-Path -LiteralPath $ReleaseDirectory).Path
$package=Read-AuditJson (Join-Path $workspace 'package.json')
$version=$package.version
if(-not $OutputDirectory){$OutputDirectory=Join-Path ([IO.Path]::GetTempPath()) ('maobu-candidate-audit-'+[Guid]::NewGuid().ToString('N'))}
$output=Assert-AuditTempPath $OutputDirectory
if(Test-Path -LiteralPath $output){throw 'Use a newly named TEMP audit directory to keep evidence distinct'}
New-Item -ItemType Directory -Path $output | Out-Null
$artifacts=Join-Path $output 'artifacts'
New-Item -ItemType Directory -Path $artifacts | Out-Null
if(-not $SevenZip){$SevenZip=(Get-Command 7z.exe -ErrorAction Stop).Source}
if(-not (Test-Path -LiteralPath $SevenZip)){throw '7-Zip is required for a read-only package inventory'}
$checks=[Collections.Generic.List[object]]::new()
$records=[Collections.Generic.List[object]]::new()
$report=[ordered]@{Kind='NEW_BUILD_CANDIDATE_ONLY';Version=$version;BuildStartedUtc=$BuildStartedUtc.ToUniversalTime().ToString('o');AuditUtc=[datetime]::UtcNow.ToString('o');Output=$output;Checks=$checks;Artifacts=$records;Boundaries=@('No installer/client was executed.','No host registry, real database, settings or cookies were changed.','Hyper-V cmdlets exist but Get-VM is inaccessible; no prepared usable VM, Sandbox, VBox, VMware or QEMU is available.','Install, upgrade, rollback, uninstall and production EXE /v1/health remain unverified in this inspection.')}

try {
    $cargo=Get-Content -LiteralPath (Join-Path $workspace 'src-tauri/Cargo.toml') -Raw -Encoding UTF8
    $tauri=Read-AuditJson (Join-Path $workspace 'src-tauri/tauri.conf.json')
    $extensionPackage=Read-AuditJson (Join-Path $workspace 'extension/package.json')
    $extensionManifest=Read-AuditJson (Join-Path $workspace 'extension/manifest.json')
    $versions=@($version,[regex]::Match($cargo,'(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value,$tauri.version,$extensionPackage.version,$extensionManifest.version)
    $report.SourceVersions=$versions
    Add-AuditCheck $checks 'Five version files and compatibility identifier' (@($versions | Where-Object {$_ -ne $version}).Count -eq 0 -and $tauri.identifier -eq 'app.lumaget.desktop') 'Root package, Cargo, Tauri, extension package and manifest must agree; legacy identifier is retained.'
    $application=Copy-AuditArtifact (Join-Path $release 'maobu-fetch.exe') (Join-Path $artifacts "maobu-fetch-v$version-portable.exe") $BuildStartedUtc
    [void]$records.Add((Get-AuditArtifact $application 'portable-executable'))
    $info=Get-AuditVersion $application.FullName
    $report.ExecutableProperties=$info
    Add-AuditCheck $checks 'EXE branding and version resources' ($info.ProductName -in @('Maobu Fetch','猫步下载器') -and $info.ProductVersion -in @($version,"$version.0") -and $info.FileVersion -in @($version,"$version.0") -and $info.OriginalFilename -eq 'maobu-fetch.exe') 'Read actual PE ProductName, ProductVersion, FileVersion and OriginalFilename.'
    $report.ApplicationPayloadComparisons=[Collections.Generic.List[object]]::new()
    $scanFiles=[Collections.Generic.List[string]]::new()
    $scanFiles.Add($application.FullName)

    foreach($kind in @('msi','nsis')) {
        $extension=if($kind -eq 'msi'){'.msi'}else{'.exe'}
        $candidates=@(Get-ChildItem -LiteralPath (Join-Path $release "bundle/$kind") -File | Where-Object {$_.Extension -eq $extension -and $_.Name.Contains("_$version"+'_')})
        if($candidates.Count -ne 1){throw "Expected one fresh $kind candidate for $version"}
        $copy=Copy-AuditArtifact $candidates[0].FullName (Join-Path $artifacts $candidates[0].Name) $BuildStartedUtc
        [void]$records.Add((Get-AuditArtifact $copy $kind))
        Add-AuditCheck $checks "$kind below 30 MB" ($copy.Length -lt 30000000) 'Strict decimal 30 MB bound; actual byte length retained.'
        $expanded=Expand-AuditArchive $SevenZip $copy.FullName (Join-Path $output "extract/$kind")
        $media=@($expanded.Entries | Where-Object {Test-AuditMediaName $_})
        $match=$null
        if($kind -eq 'msi') {
            $inventory=Get-AuditMsiInventory $copy.FullName
            $report.Msi=$inventory
            $inventory | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'msi-database-inventory.json') -Encoding utf8
            $media += @($inventory.Files | Where-Object {Test-AuditMediaName $_.Name})
            $appRecord=@($inventory.Files | Where-Object {$_.Name -eq 'maobu-fetch.exe'})
            if($appRecord.Count -eq 1){$match=@($expanded.Files | Where-Object {$_.Name -in @($appRecord[0].Id,$appRecord[0].Name)})}
            Add-AuditCheck $checks 'MSI ProductVersion' ($inventory.Properties['ProductVersion'] -eq $version) 'Read MSI Property table in read-only mode 0.'
        }else {
            $match=@($expanded.Files | Where-Object {$_.Name -eq 'maobu-fetch.exe'})
            $report.NsisParserWarnings=$expanded.ParserWarnings
            $generated=@(Get-ChildItem -LiteralPath (Join-Path $release 'nsis') -Recurse -Filter '*.nsi' -File -ErrorAction SilentlyContinue)
            $report.NsisGeneratedFileCommands=@($generated | ForEach-Object {Select-String -LiteralPath $_.FullName -Pattern '^\s*File\s' | ForEach-Object {$_.Line.Trim()}})
        }
        Add-AuditCheck $checks "$kind excludes media executables" ($media.Count -eq 0) 'Actual archive file names and MSI File table checked, not string mentions inside app code.'
        $comparison=$null
        if($null -ne $match -and $match.Count -eq 1){$comparison=Compare-AuditApplication $application.FullName $match[0].FullName $kind;[void]$report.ApplicationPayloadComparisons.Add($comparison)}
        $same=$null -ne $comparison -and $comparison.Matches
        Add-AuditCheck $checks "$kind contains the same candidate application payload" $same 'Require one complete portable UNK marker and the expected MSI/NSS marker at the same offset; only those three bytes may differ. All raw hashes are retained.'
        foreach($file in $expanded.Files){$scanFiles.Add($file.FullName)}
    }

    $zips=New-AuditExtensionZips $workspace $artifacts $version
    foreach($zip in $zips) {
        [void]$records.Add((Get-AuditArtifact $zip 'browser-extension'))
        $expanded=Expand-AuditArchive $SevenZip $zip.FullName (Join-Path $output ('extract/'+$zip.BaseName))
        Add-AuditCheck $checks ($zip.Name+' package contents') (@($expanded.Entries | Where-Object {$_ -like '*.test.js' -or (Test-AuditMediaName $_)}).Count -eq 0) 'Current source/distribution hashes and manifest version were verified before packaging.'
        foreach($file in $expanded.Files){$scanFiles.Add($file.FullName)}
    }
    Add-AuditCheck $checks 'Extension dual asset aliases are identical' ((Get-FileHash -LiteralPath $zips[0].FullName).Hash -eq (Get-FileHash -LiteralPath $zips[1].FullName).Hash) 'Versioned extension and legacy extension.zip are real identical files.'
    $hits=@(Get-AuditPathHits $scanFiles.ToArray() @($workspace,$env:USERPROFILE))
    $report.DeveloperPathHits=$hits
    Add-AuditCheck $checks 'No actual developer paths in UTF-8 or UTF-16LE' ($hits.Count -eq 0) 'Checked workspace/profile prefixes, both slash spellings and both UTF-16 alignments in app and extracted package/extension files.'
    $report.Status=if(@($checks | Where-Object {-not $_.Passed}).Count -eq 0){'STATIC_CANDIDATE_CHECKS_PASSED'}else{'FAILED'}
}catch {
    $report.Status='FAILED'
    $report.Error=$_.Exception.Message
}finally {
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $output 'release-inspection.json') -Encoding utf8
    $table=@('| Artifact | Bytes | SHA-256 |','| --- | ---: | --- |')
    foreach($item in $records){$table += "| $($item.Name) | $($item.Bytes) | $($item.Sha256) |"}
    $table | Set-Content -LiteralPath (Join-Path $output 'artifact-checksums.md') -Encoding utf8
}
[pscustomobject]@{Status=$report.Status;Output=$output;Checks=$checks.Count;Failed=@($checks | Where-Object {-not $_.Passed});Artifacts=$records} | ConvertTo-Json -Depth 6
if($report.Status -eq 'FAILED'){throw "Candidate inspection failed; see $output/release-inspection.json"}
