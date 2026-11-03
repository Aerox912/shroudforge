param(
    [string]$CargoTargetDir = 'target',
    [string]$ComponentCacheDirectory = $env:SHROUDFORGE_PROXY_CACHE_DIR,
    [string]$CacheKey = $env:SHROUDFORGE_PROXY_CACHE_KEY,
    [switch]$StandaloneEml
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$output = Join-Path $root 'build/x64'
New-Item -ItemType Directory -Force -Path $output | Out-Null
$cargoTargetPath = if ([IO.Path]::IsPathRooted($CargoTargetDir)) { [IO.Path]::GetFullPath($CargoTargetDir) } else { [IO.Path]::GetFullPath((Join-Path $root $CargoTargetDir)) }
$cachePath = $null
$fileNames = @('dbghelp.dll', 'dinput8.dll')
$proxyTarget = 'x86_64-pc-windows-msvc'
$staticFeatures = 'dbghelp-proxy/sf-static-startup,dinput8-proxy/sf-static-startup'
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
function Copy-VerifiedProxy([string]$Source, [string]$Destination, [string]$ExpectedHash) {
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
    if ((Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ExpectedHash) {
        throw "Copied Windows proxy failed validation: $Destination"
    }
}
try {
    $staticImportLibrarySha256 = $null
    if (-not $StandaloneEml) {
        $staticImportLibrary = Join-Path $output 'ShroudForgeWinmm.lib'
        if (-not (Test-Path -LiteralPath $staticImportLibrary -PathType Leaf)) {
            throw 'Build the SF winmm bootstrap first (ShroudForgeWinmm.lib is required).'
        }
        if ((Get-Item -LiteralPath $staticImportLibrary).Length -le 0) {
            throw 'The SF winmm import library is empty.'
        }
        $staticImportLibrarySha256 = (Get-FileHash -LiteralPath $staticImportLibrary -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($staticImportLibrarySha256 -cnotmatch '^[a-f0-9]{64}$') {
            throw 'The SF winmm import library hash is invalid.'
        }
    }

    if ($ComponentCacheDirectory) {
        # CacheKey from the build environment is the upstream source/toolchain
        # key. Validate it before deriving a mode-specific cache directory.
        $providedBaseKey = $CacheKey
        $actualBaseKey = & (Join-Path $PSScriptRoot 'get_component_cache_context.ps1') -Component proxies -NoExport
        if ($providedBaseKey -and ($providedBaseKey -cnotmatch '^[a-f0-9]{64}$' -or $providedBaseKey -cne $actualBaseKey)) {
            Write-Warning 'The proxy component cache key does not match the current sources and toolchain. Building from source without reading or publishing this cache.'
            $ComponentCacheDirectory = ''
        }
        $CacheKey = $actualBaseKey
        if (-not $StandaloneEml) {
            # Static ShroudForge outputs additionally bind the build mode,
            # explicit target, feature set, and exact bootstrap import-library bytes.
            $cacheKeyMaterial = "format=1`nbaseKey=$actualBaseKey`nmode=shroudforge-static-startup`ntarget=$proxyTarget`nimportLibrarySha256=$staticImportLibrarySha256`nfeatures=$staticFeatures"
            $CacheKey = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($cacheKeyMaterial))).ToLowerInvariant()
        }
    }
    if ($ComponentCacheDirectory) {
        $cacheRoot = if ([IO.Path]::IsPathRooted($ComponentCacheDirectory)) { [IO.Path]::GetFullPath($ComponentCacheDirectory) } else { [IO.Path]::GetFullPath((Join-Path $root $ComponentCacheDirectory)) }
        $cachePath = Join-Path $cacheRoot $CacheKey
        $manifestPath = Join-Path $cachePath 'manifest.json'
        $cacheValid = $false
        if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
            try {
                if ((Get-Item -LiteralPath $manifestPath).Length -gt 65536) { throw 'The proxy cache manifest is too large.' }
                $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
                if ($manifest.format -ne 1 -or $manifest.component -cne 'proxies' -or $manifest.key -cne $CacheKey -or
                    @($manifest.files).Count -ne $fileNames.Count -or
                    ((@($manifest.files.name | Sort-Object) -join ',') -cne ($fileNames -join ','))) {
                    throw 'The proxy cache manifest does not match its contract.'
                }
                foreach ($file in $manifest.files) {
                    $path = Join-Path $cachePath $file.name
                    if ($file.bytes -le 0 -or $file.sha256 -cnotmatch '^[a-f0-9]{64}$' -or
                        -not (Test-Path -LiteralPath $path -PathType Leaf) -or
                        (Get-Item -LiteralPath $path).Length -ne $file.bytes -or
                        (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $file.sha256) {
                        throw "Proxy cache file failed validation: $($file.name)"
                    }
                }
                $cacheValid = $true
            } catch {
                Write-Host "[component-cache] proxies miss: $($_.Exception.Message)"
            }
        }
        if ($cacheValid) {
            try {
                foreach ($file in $manifest.files) {
                    Copy-VerifiedProxy -Source (Join-Path $cachePath $file.name) -Destination (Join-Path $output $file.name) -ExpectedHash $file.sha256
                }
                Write-Host "[component-cache] proxies hit key=$CacheKey"
                return
            } catch {
                Write-Host "[component-cache] proxies miss: $($_.Exception.Message)"
            }
        }
        Write-Host "[component-cache] proxies build key=$CacheKey"
    }

    $priorImportDirectory = $env:SF_BOOTSTRAP_IMPORT_DIR
    try {
        $buildArguments = @(
            'build'
            '--locked'
            '--manifest-path'
            (Join-Path $root 'kfc-parser\Cargo.toml')
            '--release'
            '-p'
            'dbghelp-proxy'
            '-p'
            'dinput8-proxy'
            '--target-dir'
            $cargoTargetPath
            '--timings'
        )
        if (-not $StandaloneEml) {
            $env:SF_BOOTSTRAP_IMPORT_DIR = $output
            $buildArguments += @('--target', $proxyTarget, '--features', $staticFeatures)
        }
        & cargo @buildArguments
        if ($LASTEXITCODE -ne 0) {
            throw 'Windows proxy build failed.'
        }
    } finally {
        $env:SF_BOOTSTRAP_IMPORT_DIR = $priorImportDirectory
    }

    $releaseOutput = if ($StandaloneEml) {
        Join-Path $cargoTargetPath 'release'
    } else {
        Join-Path $cargoTargetPath "$proxyTarget/release"
    }
    foreach ($name in $fileNames) {
        $binary = Join-Path $releaseOutput $name
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Windows proxy build output is missing: $binary" }
        Copy-VerifiedProxy -Source $binary -Destination (Join-Path $output $name) -ExpectedHash (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
    }

    if ($cachePath) {
        New-Item -ItemType Directory -Force -Path $cachePath | Out-Null
        $files = foreach ($name in $fileNames) {
            $binary = Join-Path $releaseOutput $name
            $binaryHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant()
            $cachedBinary = Join-Path $cachePath $name
            Copy-VerifiedProxy -Source $binary -Destination $cachedBinary -ExpectedHash $binaryHash
            [ordered]@{
                name = $name
                bytes = (Get-Item -LiteralPath $cachedBinary).Length
                sha256 = (Get-FileHash -LiteralPath $cachedBinary -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        }
        # Publish the manifest last. An interrupted copy cannot form a cache hit.
        [ordered]@{ format = 1; component = 'proxies'; key = $CacheKey; files = @($files) } |
            ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $cachePath 'manifest.json') -Encoding utf8
        Write-Host "[component-cache] proxies saved key=$CacheKey"
    }
} finally {
    $stopwatch.Stop()
    Write-Host ('[build-timing] END windows-proxies elapsed_seconds={0:N1}' -f $stopwatch.Elapsed.TotalSeconds)
}
