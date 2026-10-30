[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$parserRoot = Join-Path $repositoryRoot 'kfc-parser'
$patchPath = Join-Path $PSScriptRoot 'parser-integration.patch'
$integratedRevisions = @(
    '742b1cb796238ebdec143842545a5a8f5fc79459'
    '2cf45e36cd4575fa3867761864a36e1883c82add'
    'c7c9b72a60ac43d5c5fc4b1ace6e9a2fdaf20934'
)
if (-not (Test-Path -LiteralPath (Join-Path $parserRoot '.git'))) {
    throw 'Initialize the kfc-parser Git submodule before preparing its local integration.'
}
$revision = & git -C $parserRoot rev-parse HEAD
if ($LASTEXITCODE -eq 0 -and $revision -in $integratedRevisions) {
    Write-Output "KFC parser integration is included in the pinned $($revision.Substring(0, 8)) submodule revision."
    exit 0
}
if ($LASTEXITCODE -ne 0 -or $revision -ne '34a156f6614f78c3340c07e4750afa400349abeb') {
    throw 'Parser preparation requires the integrated c7c9b72a, 2cf45e36 or 742b1cb7 submodule revision, or the legacy 34a156f6 revision.'
}
& git -C $parserRoot apply --reverse --check $patchPath 2>$null
if ($LASTEXITCODE -eq 0) {
    Write-Output 'KFC parser integration is already prepared.'
    exit 0
}
& git -C $parserRoot apply --check $patchPath
if ($LASTEXITCODE -ne 0) {
    throw 'The parser integration patch does not apply cleanly. Review local submodule changes before building.'
}
& git -C $parserRoot apply $patchPath
if ($LASTEXITCODE -ne 0) { throw 'Failed to prepare the local KFC parser integration.' }
Write-Output 'KFC parser integration prepared from the recorded patch.'
