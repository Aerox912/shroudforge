$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$output = Join-Path $root 'build/x64'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$bootstrapTests = Join-Path $root 'modules/entrypoints/windows-bootstrap/tests/run.ps1'
& pwsh -NoProfile -File $bootstrapTests
if ($LASTEXITCODE -ne 0) { throw 'ShroudForge Windows bootstrap tests failed.' }

$preparedGenerator = Join-Path $root 'dev/hdmap-prepared-generator/Cargo.toml'
$preparedClosure = [Environment]::GetEnvironmentVariable('HD_PREPARED_SOURCE_CLOSURE')
if ($preparedClosure -cnotmatch '^[0-9a-f]{64}$') {
    throw 'Set HD_PREPARED_SOURCE_CLOSURE to the Delivery-computed committed source closure before running prepared-generator tests.'
}
& cargo test --manifest-path $preparedGenerator --locked
if ($LASTEXITCODE -ne 0) { throw 'Prepared generator caller tests failed.' }

& cargo test --manifest-path (Join-Path $root 'Cargo.toml') --release --workspace
if ($LASTEXITCODE -ne 0) { throw 'ShroudForge workspace tests failed.' }
