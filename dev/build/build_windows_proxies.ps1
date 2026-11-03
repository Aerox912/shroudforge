param([string]$CargoTargetDir = 'target', [switch]$StandaloneEml)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$output = Join-Path $root 'build/x64'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$cargoTargetPath = if ([IO.Path]::IsPathRooted($CargoTargetDir)) { [IO.Path]::GetFullPath($CargoTargetDir) } else { [IO.Path]::GetFullPath((Join-Path $root $CargoTargetDir)) }
if (-not $StandaloneEml -and -not (Test-Path -LiteralPath (Join-Path $output 'ShroudForgeWinmm.lib') -PathType Leaf)) {
    throw 'Build the SF winmm bootstrap first (ShroudForgeWinmm.lib is required).'
}
$priorImportDirectory = $env:SF_BOOTSTRAP_IMPORT_DIR
try {
    $features = @()
    if (-not $StandaloneEml) {
        $env:SF_BOOTSTRAP_IMPORT_DIR = $output
        $features = @('--features', 'dbghelp-proxy/sf-static-startup,dinput8-proxy/sf-static-startup')
    }
    & cargo build --manifest-path (Join-Path $root 'kfc-parser\Cargo.toml') --release -p dbghelp-proxy -p dinput8-proxy --target x86_64-pc-windows-msvc --target-dir $cargoTargetPath @features
    if ($LASTEXITCODE -ne 0) { throw 'Windows proxy build failed.' }
} finally { $env:SF_BOOTSTRAP_IMPORT_DIR = $priorImportDirectory }

Copy-Item -LiteralPath (Join-Path $cargoTargetPath 'x86_64-pc-windows-msvc/release/dbghelp.dll'),(Join-Path $cargoTargetPath 'x86_64-pc-windows-msvc/release/dinput8.dll') -Destination $output -Force
