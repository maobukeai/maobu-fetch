$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$dependencies = Join-Path $repository 'src-tauri/target/debug/deps'
$probeSource = Join-Path $PSScriptRoot 'm3u8_native_probe.rs'
$executable = Join-Path ([IO.Path]::GetTempPath()) ('maobu-m3u8-regression-' + [Guid]::NewGuid().ToString('N') + '.exe')
$compilerArguments = @('--edition=2021', '--test', $probeSource, '-L', ('dependency=' + $dependencies), '-o', $executable, '-A', 'dead_code')
$cargoRegistryRoot = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'registry/src' } else { Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo/registry/src' }
foreach ($registry in (Get-ChildItem -LiteralPath $cargoRegistryRoot -Directory)) {
    foreach ($windowsTarget in (Get-ChildItem -LiteralPath $registry.FullName -Directory -Filter 'windows_x86_64_msvc-*')) {
        $nativeLibraryDirectory = Join-Path $windowsTarget.FullName 'lib'
        if (Test-Path -LiteralPath $nativeLibraryDirectory) { $compilerArguments += @('-L', ('native=' + $nativeLibraryDirectory)) }
    }
}
$jsonLibrary = Get-ChildItem -LiteralPath $dependencies -Filter 'libserde_json-*.rlib' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $jsonLibrary) { throw 'Run cargo test first; missing serde_json dependency' }
$jsonHash = $jsonLibrary.BaseName.Substring('libserde_json-'.Length)
$jsonFingerprint = Get-Content -LiteralPath (Join-Path $repository ('src-tauri/target/debug/.fingerprint/serde_json-' + $jsonHash + '/lib-serde_json.json')) -Raw | ConvertFrom-Json
$jsonSerdeCore = ($jsonFingerprint.deps | Where-Object { $_[1] -eq 'serde_core' })[3]
foreach ($dependency in @('url', 'tokio', 'tokio_util', 'reqwest', 'futures_util', 'aes', 'cbc', 'hex', 'serde', 'serde_json', 'sha2', 'tempfile', 'windows_sys')) {
    $library = Get-ChildItem -LiteralPath $dependencies -Filter ('lib' + $dependency + '-*.rlib') |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($dependency -eq 'serde') {
        # Cargo builds host/build-dependency and runtime serde with different features.
        # Match serde_json's exact serde_core fingerprint rather than mixing rlibs by mtime.
        $library = Get-ChildItem -LiteralPath $dependencies -Filter 'libserde-*.rlib' | Where-Object {
            $serdeHash = $_.BaseName.Substring('libserde-'.Length)
            $serdeFingerprintPath = Join-Path $repository ('src-tauri/target/debug/.fingerprint/serde-' + $serdeHash + '/lib-serde.json')
            $serdeFingerprint = Get-Content -LiteralPath $serdeFingerprintPath -Raw | ConvertFrom-Json
            $serdeCore = ($serdeFingerprint.deps | Where-Object { $_[1] -eq 'serde_core' })[3]
            $serdeCore -eq $jsonSerdeCore
        } | Select-Object -First 1
    }
    if (-not $library) { throw ('Run cargo test first; missing compiled dependency ' + $dependency) }
    $compilerArguments += @('--extern', ($dependency + '=' + $library.FullName))
}
& rustc @compilerArguments
if ($LASTEXITCODE -ne 0) { throw 'HLS regression harness compilation failed' }
& $executable --nocapture
if ($LASTEXITCODE -ne 0) { throw 'HLS regression harness failed' }
Write-Output ('Regression executable: ' + $executable)
