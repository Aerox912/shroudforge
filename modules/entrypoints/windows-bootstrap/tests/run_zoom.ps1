$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../../..'))
$output = Join-Path $root 'build/startup-zoom-tests'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
& (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
$native = Join-Path $root 'modules/game-runtime/native'
$common = @('/nologo','/std:c++20','/EHsc','/MT','/DSHROUDFORGE_ZOOM_TEST','/DNOMINMAX',"/I$native")
$implementation = @((Join-Path $PSScriptRoot '../startup_zoom.cpp'),(Join-Path $native 'engine-hooks/write_executable_bytes.cpp'))
Push-Location $output
try {
    & cl @common (Join-Path $PSScriptRoot 'startup_zoom_tests.cpp') @implementation /link /OUT:startup_zoom_tests.exe
    if ($LASTEXITCODE) { throw 'Zoom unit build failed' }
    & ./startup_zoom_tests.exe
    if ($LASTEXITCODE) { throw 'Zoom unit tests failed' }
    & cl @common /LD (Join-Path $PSScriptRoot 'zoom_gate_dll.cpp') @implementation /link /OUT:zoom_gate_test.dll /IMPLIB:zoom_gate_test.lib
    if ($LASTEXITCODE) { throw 'Zoom gate DLL build failed' }
    & cl /nologo /std:c++20 /EHsc /MT (Join-Path $PSScriptRoot 'startup_gate_exe.cpp') zoom_gate_test.lib /link /OUT:zoom_gate_test.exe
    if ($LASTEXITCODE) { throw 'Zoom gate EXE build failed' }
    foreach ($mode in @('success','partial','conflict')) {
        $start = [Diagnostics.ProcessStartInfo]::new((Join-Path $output 'zoom_gate_test.exe'))
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.Environment.Remove('SF_GATE_TEST_FAIL') | Out-Null
        $start.Environment.Remove('SF_ZOOM_CONFLICT') | Out-Null
        if ($mode -eq 'partial') { $start.Environment['SF_GATE_TEST_FAIL']='1' }
        if ($mode -eq 'conflict') { $start.Environment['SF_ZOOM_CONFLICT']='1' }
        $process = [Diagnostics.Process]::Start($start)
        if (-not $process.WaitForExit(10000)) { $process.Kill(); throw 'Zoom gate deadlocked' }
        $expected = if ($mode -eq 'success') {0} else {1114}
        if ($process.ExitCode -ne $expected) { throw "Zoom gate $mode returned $($process.ExitCode), expected $expected" }
        $process.Dispose()
    }
    Write-Host 'PASS 3 parked-entrypoint zoom gate cases (success, partial write, restore conflict).'
} finally { Pop-Location }
