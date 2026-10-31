$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$dependencies = Join-Path $repository 'src-tauri/target/debug/deps'
$probeSource = Join-Path $PSScriptRoot 'cloud_share_probe.rs'
$executable = Join-Path ([IO.Path]::GetTempPath()) ('maobu-cloud-probe-' + [Guid]::NewGuid().ToString('N') + '.exe')
$compilerArguments = @('--edition=2021', $probeSource, '-L', ('dependency=' + $dependencies), '-o', $executable, '-A', 'dead_code')
foreach ($dependency in @('tokio', 'reqwest', 'regex', 'serde', 'serde_json', 'url', 'hex', 'rand', 'md5', 'base64', 'tracing')) {
    $library = Get-ChildItem -LiteralPath $dependencies -Filter ('lib' + $dependency + '-*.rlib') |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $library) { throw ('Run cargo test first; missing compiled dependency ' + $dependency) }
    $compilerArguments += @('--extern', ($dependency + '=' + $library.FullName))
}
& rustc @compilerArguments
if ($LASTEXITCODE -ne 0) { throw 'Cloud audit harness compilation failed' }
& $executable
if ($LASTEXITCODE -ne 0) { throw 'Cloud audit harness execution failed' }
