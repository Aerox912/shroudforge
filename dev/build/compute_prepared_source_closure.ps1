<#
.SYNOPSIS
Create the schema-1 committed source receipt required by prepared-generator tests.

.DESCRIPTION
The receipt binds the clean root commit/tree, all three root gitlinks, and the
two Cargo lockfiles. The receipt hash is exported under every compile-time
source-closure variable used by the Main tests. Lockfile hashes cover the exact
bytes in the clean checkout; CRLF and LF checkouts can therefore produce
different lockfile hashes. No newline normalization is applied. JSON is UTF-8
without a BOM, uses LF line endings, two-space indentation, and fixed
field/path ordering.
#>
[CmdletBinding()]
param(
    [string]$RepositoryPath = (Join-Path $PSScriptRoot '..\..'),
    [string]$OutputPath = (Join-Path $PSScriptRoot '..\..\build\prepared-source-closure.schema1.json'),
    [string]$GitHubEnvPath = $env:GITHUB_ENV,
    [switch]$TestOnly
)

$ErrorActionPreference = 'Stop'

if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw 'PowerShell 7 or later is required.'
}

$root = [IO.Path]::GetFullPath($RepositoryPath)
$git = (Get-Command git -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
$utf8NoBom = [Text.UTF8Encoding]::new($false, $true)
$expectedGitlinkPaths = @(
    'kfc-parser',
    'modules/game-runtime/native/json',
    'modules/hotkeys/SDL'
)
$expectedLockfilePaths = @(
    'Cargo.lock',
    'dev/hdmap-prepared-generator/Cargo.lock'
)
$sourceClosureEnvironmentNames = @(
    'HD_PREPARED_SOURCE_CLOSURE',
    'SF_HD_FORWARD_SOURCE_CLOSURE',
    'SF_FULL_PREPARED_SOURCE_CLOSURE',
    'SF_FULL_RUNTIME_SOURCE_CLOSURE'
)

function Get-SourceClosureEnvironmentNames {
    return @($sourceClosureEnvironmentNames)
}

function Assert-SourceClosureEnvironmentBindings {
    param(
        [Parameter(Mandatory)][Collections.IDictionary]$Bindings,
        [Parameter(Mandatory)][string]$ManifestHash
    )

    if ($ManifestHash -cnotmatch '^[0-9a-f]{64}$') {
        throw 'The prepared source closure hash must be lowercase SHA-256.'
    }
    foreach ($name in $sourceClosureEnvironmentNames) {
        if (-not $Bindings.Contains($name) -or [string]$Bindings[$name] -cne $ManifestHash) {
            throw "$name must equal the persisted prepared source closure SHA-256."
        }
    }
}

function Add-SourceClosureEnvironmentBindings {
    param(
        [Parameter(Mandatory)][string]$EnvironmentPath,
        [Parameter(Mandatory)][string]$ManifestHash
    )

    if ($ManifestHash -cnotmatch '^[0-9a-f]{64}$') {
        throw 'The prepared source closure hash must be lowercase SHA-256.'
    }
    if (-not (Test-Path -LiteralPath $EnvironmentPath -PathType Leaf)) {
        throw 'The GitHub environment file does not exist.'
    }

    $environmentBytes = [IO.File]::ReadAllBytes($EnvironmentPath)
    $environmentText = $utf8NoBom.GetString($environmentBytes)
    $environmentForScan = $environmentText.TrimStart([char]0xFEFF)
    $existingNames = [Collections.Generic.List[string]]::new()
    foreach ($name in $sourceClosureEnvironmentNames) {
        $pattern = '(?im)^' + [Regex]::Escape($name) + '(?:=|<<)'
        if ([Regex]::IsMatch($environmentForScan, $pattern)) {
            $existingNames.Add($name)
        }
    }
    if ($existingNames.Count -gt 0) {
        throw ('The GitHub environment file already defines prepared-source closure binding(s): ' + ($existingNames -join ', ') + '.')
    }

    $prefix = if ($environmentText.Length -gt 0 -and -not $environmentText.EndsWith("`n", [StringComparison]::Ordinal)) { "`n" } else { '' }
    $lines = @($sourceClosureEnvironmentNames | ForEach-Object { "$_=$ManifestHash" })
    $environmentBlock = $prefix + ($lines -join "`n") + "`n"
    [IO.File]::AppendAllText(
        $EnvironmentPath,
        $environmentBlock,
        [Text.UTF8Encoding]::new($false)
    )
}

function Invoke-Git {
    param(
        [Parameter(Mandatory)][string]$RepositoryPath,
        [Parameter(Mandatory)][string[]]$Arguments
    )

    $output = & $git -C $RepositoryPath @Arguments 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "Git command failed: $($Arguments -join ' ')."
    }
    return @($output | ForEach-Object { [string]$_ })
}

function Get-Sha256Hex {
    param([Parameter(Mandatory)][byte[]]$Bytes)

    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        return [BitConverter]::ToString($hasher.ComputeHash($Bytes)).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }
}

function Get-FileSha256 {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw 'A committed lockfile is missing from the clean checkout.'
    }
    return (Get-Sha256Hex -Bytes ([IO.File]::ReadAllBytes($Path)))
}

function Assert-CleanRootStatus {
    param([string[]]$StatusLines)

    if ($StatusLines.Count -ne 0) {
        throw 'Prepared source closure requires a clean committed source tree.'
    }
}

function Assert-InitializedSubmoduleStatus {
    param([string[]]$StatusLines)

    foreach ($line in $StatusLines) {
        if ($line.Length -eq 0 -or $line[0] -ne ' ') {
            throw 'Every committed submodule must be initialized at its recorded gitlink.'
        }
    }
}

function Format-PreparedSourceClosureManifest {
    param(
        [Parameter(Mandatory)][string]$RootCommit,
        [Parameter(Mandatory)][string]$RootTree,
        [Parameter(Mandatory)][object[]]$Lockfiles,
        [Parameter(Mandatory)][object[]]$Gitlinks
    )

    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add('{')
    $lines.Add('  "lockfiles": [')
    for ($index = 0; $index -lt $Lockfiles.Count; $index++) {
        $comma = if ($index -lt ($Lockfiles.Count - 1)) { ',' } else { '' }
        $record = $Lockfiles[$index]
        $lines.Add('    {')
        $lines.Add("      `"path`": `"$($record.path)`",")
        $lines.Add("      `"sha256`": `"$($record.sha256)`"")
        $lines.Add("    }$comma")
    }
    $lines.Add('  ],')
    $lines.Add('  "schema_version": 1,')
    $lines.Add('  "source_root": {')
    $lines.Add("    `"commit`": `"$RootCommit`",")
    $lines.Add('    "gitlinks": [')
    for ($index = 0; $index -lt $Gitlinks.Count; $index++) {
        $comma = if ($index -lt ($Gitlinks.Count - 1)) { ',' } else { '' }
        $record = $Gitlinks[$index]
        $lines.Add('      {')
        $lines.Add("        `"commit`": `"$($record.commit)`",")
        $lines.Add("        `"path`": `"$($record.path)`",")
        $lines.Add("        `"tree`": `"$($record.tree)`"")
        $lines.Add("      }$comma")
    }
    $lines.Add('    ],')
    $lines.Add('    "repository": "shroudforge",')
    $lines.Add("    `"tree`": `"$RootTree`"")
    $lines.Add('  }')
    $lines.Add('}')
    return ($lines -join "`n") + "`n"
}

if ($TestOnly) {
    return
}

$topLevel = (Invoke-Git -RepositoryPath $root -Arguments @('rev-parse', '--show-toplevel') | Select-Object -First 1).Trim()
if ([IO.Path]::GetFullPath($topLevel) -ine $root) {
    throw 'RepositoryPath must identify the ShroudForge repository root.'
}

$statusLines = @(Invoke-Git -RepositoryPath $root -Arguments @('status', '--porcelain=v1', '--untracked-files=all'))
Assert-CleanRootStatus -StatusLines $statusLines

$submoduleStatus = @(Invoke-Git -RepositoryPath $root -Arguments @('submodule', 'status', '--recursive'))
Assert-InitializedSubmoduleStatus -StatusLines $submoduleStatus

$rootCommit = (Invoke-Git -RepositoryPath $root -Arguments @('rev-parse', '--verify', 'HEAD^{commit}') | Select-Object -First 1).Trim()
$rootTree = (Invoke-Git -RepositoryPath $root -Arguments @('rev-parse', '--verify', 'HEAD^{tree}') | Select-Object -First 1).Trim()
if ($rootCommit -cnotmatch '^[0-9a-f]{40}$' -or $rootTree -cnotmatch '^[0-9a-f]{40}$') {
    throw 'The committed source identity is not a canonical SHA-1 commit/tree pair.'
}

$gitlinkCommits = [Collections.Generic.Dictionary[string, string]]::new([StringComparer]::Ordinal)
$treeEntries = @(Invoke-Git -RepositoryPath $root -Arguments @('ls-tree', '-r', '--full-tree', 'HEAD'))
foreach ($entry in $treeEntries) {
    if ($entry -match '^160000 commit ([0-9a-f]{40})\t(.+)$') {
        $gitlinkCommits.Add($Matches[2], $Matches[1])
    }
}
if ($gitlinkCommits.Count -ne $expectedGitlinkPaths.Count) {
    throw 'The committed source tree has a different root gitlink inventory than schema 1.'
}

$gitlinks = [Collections.Generic.List[object]]::new()
foreach ($path in $expectedGitlinkPaths) {
    if (-not $gitlinkCommits.ContainsKey($path)) {
        throw "The committed source tree is missing expected gitlink '$path'."
    }

    $submodulePath = Join-Path $root $path
    $submoduleCommit = (Invoke-Git -RepositoryPath $submodulePath -Arguments @('rev-parse', '--verify', 'HEAD^{commit}') | Select-Object -First 1).Trim()
    if ($submoduleCommit -cne $gitlinkCommits[$path]) {
        throw "Submodule '$path' is not checked out at its committed gitlink."
    }
    if (@(Invoke-Git -RepositoryPath $submodulePath -Arguments @('status', '--porcelain=v1', '--untracked-files=all')).Count -ne 0) {
        throw "Submodule '$path' has uncommitted worktree changes."
    }

    $submoduleTree = (Invoke-Git -RepositoryPath $submodulePath -Arguments @('rev-parse', '--verify', 'HEAD^{tree}') | Select-Object -First 1).Trim()
    if ($submoduleTree -cnotmatch '^[0-9a-f]{40}$') {
        throw "Submodule '$path' has an invalid committed tree identity."
    }
    $gitlinks.Add([ordered]@{
        commit = $gitlinkCommits[$path]
        path = $path
        tree = $submoduleTree
    })
}

$lockfiles = [Collections.Generic.List[object]]::new()
foreach ($path in $expectedLockfilePaths) {
    $lockfiles.Add([ordered]@{
        path = $path
        sha256 = (Get-FileSha256 -Path (Join-Path $root $path))
    })
}

$manifestText = Format-PreparedSourceClosureManifest -RootCommit $rootCommit -RootTree $rootTree -Lockfiles $lockfiles.ToArray() -Gitlinks $gitlinks.ToArray()
$manifestBytes = $utf8NoBom.GetBytes($manifestText)
$manifestHash = Get-Sha256Hex -Bytes $manifestBytes

$fullOutputPath = if ([IO.Path]::IsPathRooted($OutputPath)) {
    [IO.Path]::GetFullPath($OutputPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path $root $OutputPath))
}
$outputDirectory = [IO.Path]::GetDirectoryName($fullOutputPath)
if (-not (Test-Path -LiteralPath $outputDirectory -PathType Container)) {
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
}
[IO.File]::WriteAllBytes($fullOutputPath, $manifestBytes)

if (-not [string]::IsNullOrWhiteSpace($GitHubEnvPath)) {
    $fullEnvPath = [IO.Path]::GetFullPath($GitHubEnvPath)
    Add-SourceClosureEnvironmentBindings -EnvironmentPath $fullEnvPath -ManifestHash $manifestHash
}

Write-Output $manifestHash
