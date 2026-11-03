[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$parserRoot = Join-Path $repositoryRoot 'kfc-parser'
$expectedRevision = 'ddeb44ecd44d80d1964fcc0d1c3fff29a1c49d54'

if (-not (Test-Path -LiteralPath (Join-Path $parserRoot '.git'))) {
    throw "Initialize the kfc-parser submodule at the repository's pinned gitlink ($($expectedRevision.Substring(0, 8))) before building."
}
$revision = & git -C $parserRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0) {
    throw 'Could not read the initialized kfc-parser submodule revision.'
}
if ($revision.Trim() -ne $expectedRevision) {
    throw "kfc-parser is at $($revision.Trim()); this release requires the pinned gitlink $expectedRevision. Update the parent repository and initialize its recorded submodule commit."
}
$status = & git -C $parserRoot status --porcelain
if ($LASTEXITCODE -ne 0) {
    throw 'Could not verify the kfc-parser working tree.'
}
if ($status) {
    throw 'The pinned kfc-parser working tree has local changes; review them before building.'
}
Write-Output "KFC parser integration is included in the clean pinned $($expectedRevision.Substring(0, 8)) submodule revision."
