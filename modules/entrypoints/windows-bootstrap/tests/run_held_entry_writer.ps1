param(
    [Parameter(Mandatory = $true)]
    [string]$ProviderPath
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$native = Join-Path $root 'modules/game-runtime/native'
$output = 'E:\Build\sf-combined-finish-20261029\hd021-integration\native-writer-tests'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$providerPath = [IO.Path]::GetFullPath($ProviderPath)
if (-not (Test-Path -LiteralPath $providerPath -PathType Leaf)) {
    throw "Native runtime provider not found: $providerPath"
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
& (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
$common = @('/nologo', '/std:c++20', '/EHsc', '/MT', '/DNOMINMAX', "/I$native")
$testDll = Join-Path $output 'held_entry_writer_test.dll'
$testExe = Join-Path $output 'held_entry_writer_host.exe'
$oldProvider = $env:SF_HELD_ENTRY_PROVIDER
$env:SF_HELD_ENTRY_PROVIDER = $providerPath
Push-Location $output
try {
    & cl @common /LD (Join-Path $PSScriptRoot 'held_entry_writer_dll.cpp') /link /OUT:$testDll /IMPLIB:held_entry_writer_test.lib
    if ($LASTEXITCODE) { throw 'Held-entry writer test DLL build failed' }
    & cl /nologo /std:c++20 /EHsc /MT (Join-Path $PSScriptRoot 'startup_gate_exe.cpp') held_entry_writer_test.lib /link /OUT:$testExe
    if ($LASTEXITCODE) { throw 'Held-entry writer host build failed' }
    & $testExe
    if ($LASTEXITCODE) { throw "Held-entry writer test failed with exit code $LASTEXITCODE" }
    Write-Host 'PASS canonical held-entry writer export: invalid proof refused, expected-byte apply and rollback succeeded, post-release proof refused.'
} finally {
    Pop-Location
    if ($null -eq $oldProvider) { Remove-Item Env:SF_HELD_ENTRY_PROVIDER -ErrorAction SilentlyContinue }
    else { $env:SF_HELD_ENTRY_PROVIDER = $oldProvider }
}
