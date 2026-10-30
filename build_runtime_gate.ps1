$ErrorActionPreference='Stop'
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs=& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
& (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
$native=Join-Path $PSScriptRoot 'modules/game-runtime/native'
$boot=Join-Path $PSScriptRoot 'modules/entrypoints/windows-bootstrap'
$out=Join-Path $PSScriptRoot 'build/runtime-zoom-gate'
New-Item -ItemType Directory -Force -Path $out | Out-Null
Push-Location $out
try {
 & cl /nologo /std:c++20 /EHsc /MT /DSHROUDFORGE_ZOOM_TEST /DNOMINMAX "/I$native" /LD (Join-Path $boot 'tests/zoom_runtime_gate_dll.cpp') (Join-Path $boot 'startup_zoom.cpp') (Join-Path $native 'engine-hooks/write_executable_bytes.cpp') /link /OUT:zoom_runtime_gate.dll /IMPLIB:zoom_runtime_gate.lib
 if($LASTEXITCODE){throw 'Runtime gate DLL build failed'}
 & cl /nologo /std:c++20 /EHsc /MT (Join-Path $boot 'tests/startup_gate_exe.cpp') zoom_runtime_gate.lib /link /OUT:zoom_runtime_gate.exe
 if($LASTEXITCODE){throw 'Runtime gate EXE build failed'}
} finally {Pop-Location}
