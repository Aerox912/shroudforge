<#
.SYNOPSIS
Verify the persisted schema-1 receipt and its prepared-test environment handoff.

.DESCRIPTION
Recomputes the receipt once and requires byte-for-byte deterministic output.
The temporary receipt is removed before returning.
#>
[CmdletBinding()]
param(
    [string]$ManifestPath = (Join-Path $PSScriptRoot '..\..\build\prepared-source-closure.schema1.json'),
    [string]$EvidenceRoot = '',
    [switch]$FixtureOnly
)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$buildRoot = [IO.Path]::GetFullPath((Join-Path $root 'build'))
$fullManifestPath = if ([IO.Path]::IsPathRooted($ManifestPath)) {
    [IO.Path]::GetFullPath($ManifestPath)
}
else {
    [IO.Path]::GetFullPath((Join-Path $root $ManifestPath))
}
$manifestDirectory = [IO.Path]::GetDirectoryName($fullManifestPath)
$separator = [IO.Path]::DirectorySeparatorChar

function Test-IsUnderDirectory {
    param([string]$Path, [string]$Directory)

    $prefix = [IO.Path]::GetFullPath($Directory).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar) + $separator
    return $Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

$helperPath = Join-Path $PSScriptRoot '..\build\compute_prepared_source_closure.ps1'
if ($FixtureOnly) {
    . $helperPath -TestOnly

    $fixtureLockfiles = @(
        [ordered]@{ path = 'Cargo.lock'; sha256 = 'a' * 64 },
        [ordered]@{ path = 'dev/hdmap-prepared-generator/Cargo.lock'; sha256 = 'b' * 64 }
    )
    $fixtureGitlinks = @(
        [ordered]@{ commit = 'a' * 40; path = 'kfc-parser'; tree = 'b' * 40 },
        [ordered]@{ commit = 'c' * 40; path = 'modules/game-runtime/native/json'; tree = 'd' * 40 },
        [ordered]@{ commit = 'e' * 40; path = 'modules/hotkeys/SDL'; tree = 'f' * 40 }
    )
    $fixtureText = Format-PreparedSourceClosureManifest `
        -RootCommit ('1' * 40) `
        -RootTree ('2' * 40) `
        -Lockfiles $fixtureLockfiles `
        -Gitlinks $fixtureGitlinks
    $expectedLines = @(
        '{',
        '  "lockfiles": [',
        '    {',
        '      "path": "Cargo.lock",',
        ('      "sha256": "' + ('a' * 64) + '"'),
        '    },',
        '    {',
        '      "path": "dev/hdmap-prepared-generator/Cargo.lock",',
        ('      "sha256": "' + ('b' * 64) + '"'),
        '    }',
        '  ],',
        '  "schema_version": 1,',
        '  "source_root": {',
        ('    "commit": "' + ('1' * 40) + '",'),
        '    "gitlinks": [',
        '      {',
        ('        "commit": "' + ('a' * 40) + '",'),
        '        "path": "kfc-parser",',
        ('        "tree": "' + ('b' * 40) + '"'),
        '      },',
        '      {',
        ('        "commit": "' + ('c' * 40) + '",'),
        '        "path": "modules/game-runtime/native/json",',
        ('        "tree": "' + ('d' * 40) + '"'),
        '      },',
        '      {',
        ('        "commit": "' + ('e' * 40) + '",'),
        '        "path": "modules/hotkeys/SDL",',
        ('        "tree": "' + ('f' * 40) + '"'),
        '      }',
        '    ],',
        '    "repository": "shroudforge",',
        ('    "tree": "' + ('2' * 40) + '"'),
        '  }',
        '}'
    )
    $expectedText = ($expectedLines -join "`n") + "`n"
    if ($fixtureText -cne $expectedText) {
        throw 'The source closure formatter did not match the schema-1 fixture bytes.'
    }
    $fixtureBytes = [Text.UTF8Encoding]::new($false).GetBytes($fixtureText)
    $fixtureHash = Get-Sha256Hex -Bytes $fixtureBytes
    if ($fixtureHash -cnotmatch '^[0-9a-f]{64}$') {
        throw 'The source closure formatter produced an invalid fixture digest.'
    }

    if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
        throw 'FixtureOnly requires EvidenceRoot so newline fixtures stay outside tracked source.'
    }
    $fixtureDirectory = if ([IO.Path]::IsPathRooted($EvidenceRoot)) {
        [IO.Path]::GetFullPath($EvidenceRoot)
    }
    else {
        [IO.Path]::GetFullPath((Join-Path $root $EvidenceRoot))
    }
    if (-not (Test-Path -LiteralPath $fixtureDirectory -PathType Container)) {
        New-Item -ItemType Directory -Force -Path $fixtureDirectory | Out-Null
    }
    $fixtureDirectoryInfo = Get-Item -LiteralPath $fixtureDirectory -Force
    if (($fixtureDirectoryInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'Newline fixture output directory must not be a reparse point.'
    }
    $fixtureSuffix = "$PID-$([guid]::NewGuid().ToString('N'))"
    $lfFixturePath = Join-Path $fixtureDirectory "prepared-source-closure-lf-$fixtureSuffix.bin"
    $crlfFixturePath = Join-Path $fixtureDirectory "prepared-source-closure-crlf-$fixtureSuffix.bin"
    $fixtureEncoding = [Text.UTF8Encoding]::new($false)
    $lfFixtureBytes = $fixtureEncoding.GetBytes('fixture' + [char]10)
    $crlfFixtureBytes = $fixtureEncoding.GetBytes('fixture' + [char]13 + [char]10)
    try {
        [IO.File]::WriteAllBytes($lfFixturePath, $lfFixtureBytes)
        [IO.File]::WriteAllBytes($crlfFixturePath, $crlfFixtureBytes)
        $lfFixtureHash = Get-FileSha256 -Path $lfFixturePath
        $crlfFixtureHash = Get-FileSha256 -Path $crlfFixturePath
        if ($lfFixtureHash -cne (Get-Sha256Hex -Bytes $lfFixtureBytes) -or
            $crlfFixtureHash -cne (Get-Sha256Hex -Bytes $crlfFixtureBytes) -or
            $lfFixtureHash -ceq $crlfFixtureHash) {
            throw 'Lockfile hashing did not follow the distinct LF and CRLF fixture bytes.'
        }
    }
    finally {
        foreach ($path in @($lfFixturePath, $crlfFixturePath)) {
            if (Test-Path -LiteralPath $path -PathType Leaf) {
                $fixtureItem = Get-Item -LiteralPath $path -Force
                if (($fixtureItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw 'A newline fixture unexpectedly became a reparse point.'
                }
                Remove-Item -LiteralPath $path -Force
            }
        }
    }

    Assert-CleanRootStatus -StatusLines @()
    $dirtySourceRejected = $false
    try {
        Assert-CleanRootStatus -StatusLines @(' M .github/workflows/build.yml', '?? dev/build/compute_prepared_source_closure.ps1')
    }
    catch {
        $dirtySourceRejected = $true
    }
    if (-not $dirtySourceRejected) {
        throw 'The source closure dirty-tree guard accepted a tracked source change.'
    }

    Assert-InitializedSubmoduleStatus -StatusLines @('  1111111111111111111111111111111111111111 kfc-parser')
    $uninitializedRejected = $false
    try {
        Assert-InitializedSubmoduleStatus -StatusLines @('-1111111111111111111111111111111111111111 kfc-parser')
    }
    catch {
        $uninitializedRejected = $true
    }
    if (-not $uninitializedRejected) {
        throw 'The source closure submodule guard accepted an uninitialized gitlink.'
    }

    Write-Output 'Prepared source closure schema, guard, and actual LF/CRLF byte fixtures verified.'
    return
}

$allowedManifestDirectory = Test-IsUnderDirectory -Path $fullManifestPath -Directory $buildRoot
if (-not $allowedManifestDirectory -and -not [string]::IsNullOrWhiteSpace($EvidenceRoot)) {
    $allowedManifestDirectory = Test-IsUnderDirectory -Path $fullManifestPath -Directory ([IO.Path]::GetFullPath($EvidenceRoot))
}
if (-not $allowedManifestDirectory) {
    throw 'ManifestPath must be inside the repository build directory or the declared evidence directory.'
}
if (-not (Test-Path -LiteralPath $fullManifestPath -PathType Leaf)) {
    throw 'The prepared source closure manifest is missing.'
}
$directoryInfo = Get-Item -LiteralPath $manifestDirectory -Force
if (($directoryInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'The prepared source closure output directory must not be a reparse point.'
}

$manifestBytes = [IO.File]::ReadAllBytes($fullManifestPath)
if ($manifestBytes.Length -lt 2 -or $manifestBytes[$manifestBytes.Length - 1] -ne 10 -or $manifestBytes[$manifestBytes.Length - 2] -eq 10) {
    throw 'The manifest must end with exactly one LF newline.'
}
if ($manifestBytes -contains [byte]13) {
    throw 'The manifest must use LF line endings.'
}
if ($manifestBytes.Length -ge 3 -and $manifestBytes[0] -eq 0xEF -and $manifestBytes[1] -eq 0xBB -and $manifestBytes[2] -eq 0xBF) {
    throw 'The manifest must not contain a UTF-8 byte order mark.'
}
$utf8 = [Text.UTF8Encoding]::new($false, $true)
$manifestText = $utf8.GetString($manifestBytes)
$manifestHash = [BitConverter]::ToString([Security.Cryptography.SHA256]::HashData($manifestBytes)).Replace('-', '').ToLowerInvariant()
if ($manifestHash -cnotmatch '^[0-9a-f]{64}$' -or $env:HD_PREPARED_SOURCE_CLOSURE -cne $manifestHash) {
    throw 'HD_PREPARED_SOURCE_CLOSURE does not match the persisted manifest SHA-256.'
}

$manifest = $manifestText | ConvertFrom-Json -Depth 8
if ($manifest.schema_version -ne 1 -or $manifest.source_root.repository -cne 'shroudforge') {
    throw 'The prepared source closure manifest has an unsupported schema or repository identity.'
}
$expectedLockfiles = @('Cargo.lock', 'dev/hdmap-prepared-generator/Cargo.lock')
$actualLockfiles = @($manifest.lockfiles | ForEach-Object { $_.path })
if (($actualLockfiles -join "`n") -cne ($expectedLockfiles -join "`n")) {
    throw 'The manifest lockfile inventory is not canonical.'
}
foreach ($lockfile in $manifest.lockfiles) {
    if ($lockfile.sha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw "The committed lockfile hash is invalid for '$($lockfile.path)'."
    }
}

$expectedGitlinks = @('kfc-parser', 'modules/game-runtime/native/json', 'modules/hotkeys/SDL')
$actualGitlinks = @($manifest.source_root.gitlinks | ForEach-Object { $_.path })
if (($actualGitlinks -join "`n") -cne ($expectedGitlinks -join "`n")) {
    throw 'The manifest gitlink inventory is not canonical.'
}
foreach ($gitlink in $manifest.source_root.gitlinks) {
    if ($gitlink.commit -cnotmatch '^[0-9a-f]{40}$' -or $gitlink.tree -cnotmatch '^[0-9a-f]{40}$') {
        throw "The committed gitlink identity is invalid for '$($gitlink.path)'."
    }
}

$temporaryManifestPath = Join-Path $manifestDirectory ("prepared-source-closure-repro-$PID-$([guid]::NewGuid().ToString('N')).json")
try {
    $reproducedHash = & $helperPath -RepositoryPath $root -OutputPath $temporaryManifestPath -GitHubEnvPath ''
    if ($reproducedHash -cnotmatch '^[0-9a-f]{64}$' -or -not (Test-Path -LiteralPath $temporaryManifestPath -PathType Leaf)) {
        throw 'The source closure helper did not produce a valid reproducible receipt.'
    }
    $reproducedBytes = [IO.File]::ReadAllBytes($temporaryManifestPath)
    if ($reproducedHash -cne $manifestHash -or [Convert]::ToBase64String($reproducedBytes) -cne [Convert]::ToBase64String($manifestBytes)) {
        throw 'The source closure receipt is not deterministic.'
    }
}
finally {
    if (Test-Path -LiteralPath $temporaryManifestPath -PathType Leaf) {
        $temporaryItem = Get-Item -LiteralPath $temporaryManifestPath -Force
        if (($temporaryItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'The temporary receipt unexpectedly became a reparse point.'
        }
        Remove-Item -LiteralPath $temporaryManifestPath -Force
    }
}

Write-Output "Prepared source closure verified: $manifestHash"
