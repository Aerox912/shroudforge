$ErrorActionPreference='Stop'
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs=& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
& (Join-Path $vs 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
$native=Join-Path $PSScriptRoot 'modules/game-runtime/native'
$boot=Join-Path $PSScriptRoot 'modules/entrypoints/windows-bootstrap'
Push-Location (Join-Path $PSScriptRoot 'build/startup-zoom-tests')
try {
 & cl /nologo /std:c++20 /EHsc /MT /DSHROUDFORGE_ZOOM_TEST /DNOMINMAX "/I$native" (Join-Path $boot 'tests/zoom_retained_image.cpp') (Join-Path $boot 'startup_zoom.cpp') (Join-Path $native 'engine-hooks/write_executable_bytes.cpp') /link /OUT:zoom_retained_image.exe
 if($LASTEXITCODE){throw 'Retained fixture build failed'}
 & ./zoom_retained_image.exe 'E:/Build/esm-realmap-startup-r9-20261028/inputs/enshrouded.exe'
 if($LASTEXITCODE){throw 'Retained image validation failed'}
} finally {Pop-Location}
