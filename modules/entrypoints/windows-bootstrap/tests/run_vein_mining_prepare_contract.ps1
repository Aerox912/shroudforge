param(
    [string]$OutputDirectory = 'E:\Build\sf-combined-finish-20261029\vein-startup-demand95\bridge-contract-tests-ownership-gate-r1'
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$native = Join-Path $root 'modules/game-runtime/native'
$bootstrap = Join-Path $root 'modules/entrypoints/windows-bootstrap'
$output = [IO.Path]::GetFullPath($OutputDirectory)
if ([IO.Directory]::Exists($output) -and
    (Get-ChildItem -Force -LiteralPath $output | Measure-Object).Count -gt 0) {
    throw "Test output is not empty: $output"
}
New-Item -ItemType Directory -Force -Path $output | Out-Null

$lifecycle = Get-Content -Raw -LiteralPath (Join-Path $bootstrap 'runtime_lifecycle.cpp')
$proofAt = $lifecycle.IndexOf('StartupGate::CaptureHeldEntryProof()', [StringComparison]::Ordinal)
$demandAt = $lifecycle.IndexOf('shroudforge_startup_vein_mining_demand', [StringComparison]::Ordinal)
$nativeInitAt = $lifecycle.IndexOf('EcsRuntime::Initialize(', [StringComparison]::Ordinal)
$debugUiAt = $lifecycle.IndexOf('start_debug_console(root)', [StringComparison]::Ordinal)
$runtimeCreateAt = $lifecycle.IndexOf('void* handle = create(root.c_str())', [StringComparison]::Ordinal)
if ($proofAt -lt 0 -or $demandAt -le $proofAt -or $nativeInitAt -le $demandAt -or
    $debugUiAt -le $nativeInitAt -or $runtimeCreateAt -le $nativeInitAt) {
    throw 'Vein demand must follow held-entry proof, precede provider initialization, UI, and runtime creation'
}
if ($lifecycle -notmatch '(?s)if\s*\(is_dedicated_server_process\(\)\)\s*\{\s*const auto read_vein_mining_demand[\s\S]*?GetProcAddress\(runtime,\s*"shroudforge_startup_vein_mining_demand"\)') {
    throw 'Rust Vein demand export must only be resolved and called for the actual dedicated-server process'
}
if ((Get-Content -Raw -LiteralPath (Join-Path $native 'abi/runtime_api.h')) -match 'KfcRuntimePrepareVeinMining') {
    throw 'Vein mining preparation must remain a private dynamic export outside the public runtime ABI'
}
$demandRust = Get-Content -Raw -LiteralPath (Join-Path $root 'modules/mod-runtime/session/read_native_plugin_declaration.rs')
if (-not $demandRust.Contains('const MOD_ID: &str = "sf-vein-mining";') -or
    -not $demandRust.Contains('info.enabled') -or
    -not $demandRust.Contains('info.target.supports_process(true)') -or
    -not $demandRust.Contains('Capability::Runtime') -or
    -not $demandRust.Contains('environment.plan_report_detailed(true, crate::API_VERSION)') -or
    -not $demandRust.Contains('environment.runtime_plan_report_detailed(true, crate::API_VERSION)')) {
    throw 'Vein demand must follow the canonical enabled server runtime package plan and capability'
}
$embedded = Get-Content -Raw -LiteralPath (Join-Path $root 'modules/entrypoints/loader/embedded.rs')
if (-not $embedded.Contains('shroudforge_startup_vein_mining_demand') -or
    -not $embedded.Contains('root.join("enshrouded_server.exe")') -or
    -not $embedded.Contains('shroudforge_api::vein_mining_startup_demand')) {
    throw 'Rust bootstrap export must route only the server-root request through the canonical demand API'
}
$bridge = Get-Content -Raw -LiteralPath (Join-Path $bootstrap 'runtime_bridge.cpp')
$initializeAt = $bridge.IndexOf('const auto initialized = initialize();', [StringComparison]::Ordinal)
$ownershipAt = $bridge.IndexOf('GetProcAddress(provider, "KfcRuntimePreparePlayerOwnership")', [StringComparison]::Ordinal)
$ownershipResultAt = $bridge.IndexOf('player_ownership_prepared = prepare_player_ownership(held_entry_proof);', [StringComparison]::Ordinal)
$ownershipGateAt = $bridge.IndexOf('else if (vein_mining_demanded && !player_ownership_prepared)', [StringComparison]::Ordinal)
$veinAt = $bridge.IndexOf('GetProcAddress(provider, "KfcRuntimePrepareVeinMining")', [StringComparison]::Ordinal)
$overlayAt = $bridge.IndexOf('GetProcAddress(provider, "KfcRuntimePrepareOverlay")', [StringComparison]::Ordinal)
if ($initializeAt -lt 0 -or $ownershipAt -le $initializeAt -or $ownershipResultAt -le $ownershipAt -or
    $ownershipGateAt -le $ownershipResultAt -or $veinAt -le $ownershipGateAt -or $overlayAt -le $veinAt -or
    -not $bridge.Contains('player-ownership-prepare-export-unavailable') -or
    -not $bridge.Contains('player-ownership-preparation-failed')) {
    throw 'Demanded Vein preparation must require successful player-ownership preparation before resolving the private Vein export'
}
Write-Host 'PASS Rust canonical-plan, dedicated-server routing, ownership-success gate, and private export-order source contracts.'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
& (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
$common = @('/nologo', '/std:c++20', '/EHsc', '/MT', '/DWIN32_LEAN_AND_MEAN', '/DNOMINMAX', "/I$native")
$providerSource = Join-Path $PSScriptRoot 'vein_mining_prepare_contract_dll.cpp'
$hostSource = Join-Path $PSScriptRoot 'vein_mining_prepare_contract_exe.cpp'
$bridgeSource = Join-Path $bootstrap 'runtime_bridge.cpp'
$providerWithExport = Join-Path $output 'provider-with-vein-export.dll'
$providerWithoutExport = Join-Path $output 'provider-without-vein-export.dll'
$providerWithoutOwnership = Join-Path $output 'provider-without-player-ownership-export.dll'
$hostExe = Join-Path $output 'vein-mine-bridge-contract.exe'
Push-Location $output
try {
    & cl @common '/DKFC_RUNTIME_BUILD' '/DVEIN_PREPARE_EXPORT=1' /LD $providerSource /link /OUT:$providerWithExport /IMPLIB:provider-with-vein-export.lib
    if ($LASTEXITCODE) { throw 'Private Vein provider fixture build failed' }
    & cl @common '/DKFC_RUNTIME_BUILD' '/DVEIN_PREPARE_EXPORT=0' /LD $providerSource /link /OUT:$providerWithoutExport /IMPLIB:provider-without-vein-export.lib
    if ($LASTEXITCODE) { throw 'Missing-export provider fixture build failed' }
    & cl @common '/DKFC_RUNTIME_BUILD' '/DVEIN_PREPARE_EXPORT=1' '/DPLAYER_OWNERSHIP_EXPORT=0' /LD $providerSource /link /OUT:$providerWithoutOwnership /IMPLIB:provider-without-player-ownership-export.lib
    if ($LASTEXITCODE) { throw 'Missing player-ownership export fixture build failed' }
    & cl @common $hostSource $bridgeSource /link /OUT:$hostExe
    if ($LASTEXITCODE) { throw 'Runtime bridge contract host build failed' }

    $cases = @(
        @{ name='qualified'; provider=$providerWithExport },
        @{ name='no-demand'; provider=$providerWithExport },
        @{ name='no-demand-ownership-refused'; provider=$providerWithExport },
        @{ name='client'; provider=$providerWithExport },
        @{ name='unavailable'; provider=$providerWithExport },
        @{ name='rejected'; provider=$providerWithExport },
        @{ name='ownership-refused'; provider=$providerWithExport },
        @{ name='ownership-missing'; provider=$providerWithoutOwnership },
        @{ name='missing-export'; provider=$providerWithoutExport },
        @{ name='missing-provider'; provider=$null }
    )
    foreach ($case in $cases) {
        $caseRoot = Join-Path $output $case.name
        $providerDirectory = Join-Path $caseRoot 'shroudforge'
        New-Item -ItemType Directory -Force -Path $providerDirectory | Out-Null
        Copy-Item -LiteralPath $hostExe -Destination (Join-Path $caseRoot 'vein-mine-bridge-contract.exe')
        if ($case.provider) {
            Copy-Item -LiteralPath $case.provider -Destination (Join-Path $providerDirectory 'kfc-runtime.dll')
        }
        $runner = Join-Path $caseRoot 'vein-mine-bridge-contract.exe'
        & $runner $case.name
        if ($LASTEXITCODE) { throw "Bridge contract case '$($case.name)' failed with exit code $LASTEXITCODE" }
    }
    Write-Host 'PASS inert provider contract: qualified demand, no-op demand after ownership refusal, client no-call, unavailable no-call, Vein refusal, player-ownership refusal and missing-export gates, missing Vein export, and optional provider absence.'
} finally {
    Pop-Location
}
