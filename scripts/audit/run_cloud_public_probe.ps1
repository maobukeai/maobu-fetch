param([Parameter(Mandatory=$true)][string]$Samples,[Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference='Stop'
$repository=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$dependencies=Join-Path $repository 'src-tauri/target/debug/deps'
$executable=Join-Path ([IO.Path]::GetTempPath()) ('maobu-cloud-public-'+[Guid]::NewGuid().ToString('N')+'.exe')
$compilerArguments=@('--edition=2021',(Join-Path $PSScriptRoot 'cloud_public_probe.rs'),'-L',('dependency='+$dependencies),'-o',$executable,'-A','dead_code','-C','link-arg=/STACK:16777216')
$names=@('tokio','tokio_util','futures_util','reqwest','regex','serde','serde_json','url','hex','rand','md5','sha2','base64','tracing')
$graph=& python (Join-Path $PSScriptRoot 'cloud_probe_dependencies.py') $repository @names
if($LASTEXITCODE -ne 0){throw 'Cannot identify coherent application dependencies'}
$libraries=$graph | ConvertFrom-Json
foreach($dependency in $names) {
  $compilerArguments+=@('--extern',($dependency+'='+$libraries.$dependency))
}
& rustc @compilerArguments
if($LASTEXITCODE -ne 0){throw 'Current production cloud module probe compilation failed'}
& $executable $Samples $Output
if($LASTEXITCODE -ne 0){throw 'Cloud public probe execution failed'}
