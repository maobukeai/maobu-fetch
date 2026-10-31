$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
# Directory sources now share the production scoped request builder. Compile the real crate so
# this regression cannot substitute a separate Cookie/redirect/client implementation.
& cargo test --manifest-path (Join-Path $repository 'src-tauri/Cargo.toml') --lib pagination
if ($LASTEXITCODE -ne 0) { throw 'Production cloud pagination regression failed' }
