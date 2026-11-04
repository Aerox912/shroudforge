param([Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference='Stop'
$Output=[IO.Path]::GetFullPath($Output)
if(-not $Output.StartsWith('E:\',[StringComparison]::OrdinalIgnoreCase)){throw 'E: output required'}
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$vs='C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/Common7/Tools/Launch-VsDevShell.ps1'
& $vs -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
Push-Location $Output
try {
    & cl /nologo /std:c++20 /EHsc /W4 /WX /MT (Join-Path $PSScriptRoot 'greeneye_entry_tests.cpp') /Fe:greeneye_entry_tests.exe /Fo:greeneye_entry_tests.obj
    if($LASTEXITCODE){throw 'Greeneye inert gate fixture build failed'}
    foreach($case in 0..8) {
        & (Join-Path $Output 'greeneye_entry_tests.exe') $case
        $expected=if($case -in @(0,1,5)){0}else{1114}
        if($LASTEXITCODE -ne $expected){throw "Scenario $case returned $LASTEXITCODE expected $expected"}
        Write-Host "Scenario $case expected exit $expected"
    }
} finally {Pop-Location}
