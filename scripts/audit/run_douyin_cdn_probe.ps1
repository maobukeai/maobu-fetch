param([Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference='Stop'
$repository=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$dependencies=Join-Path $repository 'src-tauri/target/debug/deps'
$build=Join-Path ([IO.Path]::GetTempPath()) ('maobu-douyin-cdn-build-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $build | Out-Null
# A rustc #[path] file imports child modules relative to its directory. Preserve the
# exact production bytes under a conventional mod.rs tree in this isolated TEMP build.
$platformSource=Join-Path $repository 'src-tauri/src/media_platforms'
$platformCopy=Join-Path $build 'media_platforms'
Copy-Item -LiteralPath $platformSource -Destination $platformCopy -Recurse
Copy-Item -LiteralPath (Join-Path $repository 'src-tauri/src/media_platforms.rs') -Destination (Join-Path $platformCopy 'mod.rs')
$source=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'douyin_cdn_probe.rs'))
$source=$source.Replace('../../src-tauri/src/media_platforms.rs',((Join-Path $platformCopy 'mod.rs').Replace([char]92,[char]47)))
$source=$source.Replace('../../src-tauri/src/media_cookies/scope.rs',((Join-Path $repository 'src-tauri/src/media_cookies/scope.rs').Replace([char]92,[char]47)))
$main=Join-Path $build 'main.rs'
[IO.File]::WriteAllText($main,$source,[Text.UTF8Encoding]::new($false))
if((Get-FileHash -LiteralPath (Join-Path $platformCopy 'mod.rs')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $repository 'src-tauri/src/media_platforms.rs')).Hash){throw 'Production module changed during isolated copy'}
$executable=Join-Path $build 'diagnostic.exe'
$compilerArguments=@('--edition=2021',$main,'-L',('dependency='+$dependencies),'-o',$executable,'-A','dead_code','-A','unused_imports')
$names=@('tokio','futures_util','reqwest','regex','serde','serde_json','url','hex','sha2','tracing')
$graph=& python (Join-Path $PSScriptRoot 'cloud_probe_dependencies.py') $repository @names
if($LASTEXITCODE -ne 0){throw 'Cannot identify coherent application dependencies'}
$libraries=$graph | ConvertFrom-Json
foreach($dependency in $names){$compilerArguments+=@('--extern',($dependency+'='+$libraries.$dependency))}
& rustc @compilerArguments
if($LASTEXITCODE -ne 0){throw 'Current production native Douyin probe compilation failed'}
& $executable $Output
if($LASTEXITCODE -ne 0){throw 'Bounded native Douyin diagnostic failed'}
