param(
    [Parameter(Mandatory = $true)]
    [string]$ProviderPath,
    [string]$OutputDirectory = 'E:\Build\sf-shared-player-callback-20261030-r1\wiring-tests'
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$native = Join-Path $root 'modules/game-runtime/native'
$providerPath = [IO.Path]::GetFullPath($ProviderPath)
$output = [IO.Path]::GetFullPath($OutputDirectory)
if (-not (Test-Path -LiteralPath $providerPath -PathType Leaf)) {
    throw "Native runtime provider not found: $providerPath"
}
if ([IO.Directory]::Exists($output) -and
    (Get-ChildItem -Force -LiteralPath $output | Measure-Object).Count -gt 0) {
    throw "Test output is not empty: $output"
}
New-Item -ItemType Directory -Force -Path $output | Out-Null

$bridgePath = Join-Path $PSScriptRoot '../runtime_bridge.cpp'
$bridge = Get-Content -Raw -LiteralPath $bridgePath
$initializeAt = $bridge.IndexOf('const auto initialized = initialize();', [StringComparison]::Ordinal)
$prepareAt = $bridge.IndexOf('GetProcAddress(provider, "KfcRuntimePreparePlayerOwnership")', [StringComparison]::Ordinal)
$guardAt = $bridge.IndexOf('if (initialized && held_entry_proof && prepare_player_ownership)', [StringComparison]::Ordinal)
$overlayAt = $bridge.IndexOf('GetProcAddress(provider, "KfcRuntimePrepareOverlay")', [StringComparison]::Ordinal)
if ($initializeAt -lt 0 -or $prepareAt -le $initializeAt -or
    $guardAt -le $prepareAt -or $overlayAt -le $guardAt) {
    throw 'Player-ownership preparation must follow successful provider initialization and precede overlay preparation'
}
$optionalBlock = $bridge.Substring($guardAt, $overlayAt - $guardAt)
if ($optionalBlock -notmatch 'catch \(\.\.\.\) \{ prepared = false; \}' -or
    $optionalBlock -notmatch 'optional native peer adapter remains unavailable' -or
    $optionalBlock -match 'return false') {
    throw 'Missing export, initialization failure, or preparation refusal must remain optional'
}

$provider = Get-Content -Raw -LiteralPath (Join-Path $native 'host/manage_runtime_lifecycle.cpp')
if ($provider -notmatch 'KfcRuntimePreparePlayerOwnership[\s\S]*?CompleteCallbackPlayerWorldPreparation\([\s\S]*?RunHeldEntryPreparation\(\s*\*proof,\s*&prepare_player_ownership,\s*nullptr\)' -or
    $provider -notmatch 'prepare_player_ownership[\s\S]*?PrepareServiceRetirement\(token\)\s*&&\s*PlayerOwnership::PrepareCallbackPlayerWorld\(token\)') {
    throw 'Provider export must use the existing held-entry coordinator and retirement preparation'
}
if ((Get-Content -Raw -LiteralPath (Join-Path $native 'abi/runtime_api.h')) -match 'KfcRuntimePreparePlayerOwnership') {
    throw 'Player-ownership bootstrap export must not enter the public runtime ABI'
}
$admission = Get-Content -Raw -LiteralPath (Join-Path $native 'player-ownership/service_lifetime_admission.h')
if ($admission -notmatch 'service_lifetime_reviewed\(\) noexcept \{\s*return true;\s*\}') {
    throw 'The independently accepted production review policy must remain enabled'
}
$service = Get-Content -Raw -LiteralPath (Join-Path $native 'player-ownership/service_lifetime.cpp')
if ($service -notmatch '(?s)#else.*?phase\{Phase::Unprepared\}') {
    throw 'Enabled review policy must not replace the production cold Unprepared state'
}

$lifecycle = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '../runtime_lifecycle.cpp')
$nativeInitAt = $lifecycle.IndexOf('EcsRuntime::Initialize(', [StringComparison]::Ordinal)
$debugUiAt = $lifecycle.IndexOf('start_debug_console(root)', [StringComparison]::Ordinal)
$modUiAt = $lifecycle.IndexOf('start_modloader_ui(root)', [StringComparison]::Ordinal)
$assetPrepareAt = $lifecycle.IndexOf('prepare_startup(root.c_str())', [StringComparison]::Ordinal)
if ($nativeInitAt -lt 0 -or $debugUiAt -le $nativeInitAt -or
    $modUiAt -le $nativeInitAt -or $assetPrepareAt -le $modUiAt) {
    throw 'Bootstrap provider initialization must remain before UI and startup asset preparation'
}
if ($lifecycle -notmatch 'zoom_demand == 0 && zoom_result != StartupZoom::Result::Disabled' -or
    $lifecycle -notmatch 'zoom_demand == 1 && zoom_result != StartupZoom::Result::Applied') {
    throw 'Existing disabled and required Extended Zoom startup semantics changed'
}
Write-Host 'PASS source order and optional preparation contracts.'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
& (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
$common = @('/nologo', '/std:c++20', '/EHsc', '/MT', '/DNOMINMAX', "/I$native")
$testDll = Join-Path $output 'player_ownership_wiring_test.dll'
$testExe = Join-Path $output 'player_ownership_wiring_host.exe'
$previousProvider = $env:SF_PLAYER_OWNERSHIP_PROVIDER
$env:SF_PLAYER_OWNERSHIP_PROVIDER = $providerPath
Push-Location $output
try {
    & cl @common /LD (Join-Path $PSScriptRoot 'player_ownership_wiring_dll.cpp') /link /OUT:$testDll /IMPLIB:player_ownership_wiring_test.lib
    if ($LASTEXITCODE) { throw 'Player-ownership wiring test DLL build failed' }
    & cl /nologo /std:c++20 /EHsc /MT (Join-Path $PSScriptRoot 'player_ownership_wiring_exe.cpp') player_ownership_wiring_test.lib /link /OUT:$testExe
    if ($LASTEXITCODE) { throw 'Player-ownership wiring host build failed' }
    & $testExe
    if ($LASTEXITCODE) { throw "Player-ownership wiring fixture failed with exit code $LASTEXITCODE" }
    Write-Host 'PASS actual provider export: invalid and released proofs refused; unprepared/non-game provider remained optional despite accepted review policy.'
} finally {
    Pop-Location
    if ($null -eq $previousProvider) { Remove-Item Env:SF_PLAYER_OWNERSHIP_PROVIDER -ErrorAction SilentlyContinue }
    else { $env:SF_PLAYER_OWNERSHIP_PROVIDER = $previousProvider }
}
