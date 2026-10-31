param([switch]$Cli)
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$dependencies = Join-Path $repository 'src-tauri/target/debug/deps'
$source = Join-Path $PSScriptRoot 'media_muxer_native_probe.rs'
$executable = Join-Path ([IO.Path]::GetTempPath()) ('maobu-muxer-probe-' + [Guid]::NewGuid().ToString('N') + '.exe')
$compilerArguments = @('--edition=2021',$source,'-L',('dependency='+$dependencies),'-o',$executable,'-A','dead_code')
if (-not $Cli) { $compilerArguments += '--test' }
$registryRoot = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'registry/src' } else { Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo/registry/src' }
foreach ($registry in (Get-ChildItem -LiteralPath $registryRoot -Directory)) {
    foreach ($windowsTarget in (Get-ChildItem -LiteralPath $registry.FullName -Directory -Filter 'windows_x86_64_msvc-*')) {
        $nativeLibrary = Join-Path $windowsTarget.FullName 'lib'
        if (Test-Path -LiteralPath $nativeLibrary) { $compilerArguments += @('-L',('native='+$nativeLibrary)) }
    }
}
foreach ($dependency in @('tokio','tokio_util','uuid','sha2','tempfile','windows_sys','tracing')) {
    $library = Get-ChildItem -LiteralPath $dependencies -Filter ('lib'+$dependency+'-*.rlib') | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $library) { throw ('missing compiled dependency '+$dependency) }
    $compilerArguments += @('--extern',($dependency+'='+$library.FullName))
}
& rustc @compilerArguments
if ($LASTEXITCODE -ne 0) { throw 'fMP4 production probe compilation failed' }
if (-not $Cli) {
    & $executable --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'fMP4 production behavior regression failed' }
}
Write-Output ('Probe executable: '+$executable)
