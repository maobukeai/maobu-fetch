param([Parameter(Mandatory=$true)][string]$InputDirectory,[Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference='Stop'
$repository=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$build=Join-Path ([IO.Path]::GetTempPath()) ('maobu-douyin-worker-build-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $build | Out-Null
$source=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'douyin_worker_probe.rs'))
foreach($relative in @('media_cookies/scope.rs','manager/request_headers.rs','manager/http_redirect.rs')){
    $source=$source.Replace(('../../src-tauri/src/'+$relative),((Join-Path $repository ('src-tauri/src/'+$relative)).Replace([char]92,[char]47)))
}
$main=Join-Path $build 'main.rs'
[IO.File]::WriteAllText($main,$source,[Text.UTF8Encoding]::new($false))
$names=@('tokio','tokio_util','futures_util','reqwest','serde_json','url','hex','sha2')
$graph=& python (Join-Path $PSScriptRoot 'cloud_probe_dependencies.py') $repository @names
if($LASTEXITCODE -ne 0){throw 'Cannot identify current application dependencies'}
$libraries=$graph | ConvertFrom-Json
$executable=Join-Path $build 'diagnostic.exe'
$arguments=@('--edition=2021',$main,'-L',('dependency='+(Join-Path $repository 'src-tauri/target/debug/deps')),'-o',$executable,'-A','dead_code')
foreach($name in $names){$arguments+=@('--extern',($name+'='+$libraries.$name))}
& rustc @arguments
if($LASTEXITCODE -ne 0){throw 'Production worker adapter diagnostic compilation failed'}
& $executable $InputDirectory $Output
if($LASTEXITCODE -ne 0){throw 'Bounded worker adapter diagnostic failed'}
