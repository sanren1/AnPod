<#
.SYNOPSIS
    Produces the x64 portable package for AnPod.

.DESCRIPTION
    Publishes the WPF application, folds in the runtime dependencies from deps/,
    copies the licence and notice files, and writes a zip. Missing dependencies
    are reported as warnings rather than hard failures so a partial build is
    still inspectable.

    deps/ must contain (see deps/README.md):
        audio_capture.dll   from packaging/build-native.ps1
        adb.exe             from Android SDK Platform-Tools
        AdbWinApi.dll       from Android SDK Platform-Tools
        AdbWinUsbApi.dll    from Android SDK Platform-Tools
        server              from packaging/build-android.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File packaging\publish.ps1
#>
[CmdletBinding()]
param(
    [string]$Version = '1.0.0',

    [switch]$SkipTests,

    [switch]$SkipZip
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$depsDir = Join-Path $root 'deps'
$artifactsDir = Join-Path $root 'artifacts'
$publishDir = Join-Path $artifactsDir 'publish'
$packageDir = Join-Path $artifactsDir "AnPod-$Version-win-x64"
$zipPath = Join-Path $artifactsDir "AnPod-$Version-win-x64.zip"

function Get-DotNet {
    $resolved = Get-Command 'dotnet' -ErrorAction SilentlyContinue
    if (-not $resolved) {
        throw 'dotnet was not found on PATH. Install the .NET 8 SDK.'
    }
    return $resolved.Source
}

$dotnet = Get-DotNet

if (-not $SkipTests) {
    $testProject = Join-Path $root 'tests\AnPod.Core.Tests\AnPod.Core.Tests.csproj'
    if (Test-Path -LiteralPath $testProject -PathType Leaf) {
        Write-Host 'Running core checks...' -ForegroundColor Cyan
        # No --nologo here: `dotnet run` forwards unrecognised options to the
        # application, so it would arrive in the test program's args.
        & $dotnet run --project $testProject -c Release
        if ($LASTEXITCODE -ne 0) {
            throw "Core checks failed with exit code $LASTEXITCODE."
        }
    }
    else {
        Write-Warning 'Core tests are not included in the public repository checkout; continuing with the product build.'
    }
}

Write-Host 'Publishing AnPod.Desktop (win-x64)...' -ForegroundColor Cyan

& $dotnet publish (Join-Path $root 'src\AnPod.Desktop\AnPod.Desktop.csproj') `
    -c Release `
    -r win-x64 `
    --self-contained false `
    -o $publishDir `
    -v minimal `
    --nologo

if ($LASTEXITCODE -ne 0) {
    throw "dotnet publish failed with exit code $LASTEXITCODE."
}

# ---------------------------------------------------------------- dependencies

$requiredFiles = @(
    'audio_capture.dll',
    'adb.exe',
    'AdbWinApi.dll',
    'AdbWinUsbApi.dll',
    'server'
)

$missing = @()
foreach ($name in $requiredFiles) {
    $source = Join-Path $depsDir $name
    if (Test-Path $source) {
        Copy-Item -Path $source -Destination (Join-Path $publishDir $name) -Force
    }
    else {
        $missing += $name
    }
}

if ($missing.Count -gt 0) {
    Write-Warning ("Missing runtime dependencies: {0}" -f ($missing -join ', '))
    Write-Warning 'The application will start but report the missing piece at runtime.'
}

# ---------------------------------------------------------------- legal files

Copy-Item -Path (Join-Path $root 'README.md') -Destination $publishDir -Force
Copy-Item -Path (Join-Path $root 'THIRD_PARTY_NOTICES.md') -Destination $publishDir -Force

$licenseTarget = Join-Path $publishDir 'LICENSES'
New-Item -ItemType Directory -Path $licenseTarget -Force | Out-Null
Copy-Item -Path (Join-Path $root 'LICENSES\*') -Destination $licenseTarget -Force

$upstreamTarget = Join-Path $publishDir 'upstream'
New-Item -ItemType Directory -Path $upstreamTarget -Force | Out-Null
Copy-Item -Path (Join-Path $root 'upstream\*') -Destination $upstreamTarget -Force

# The platform-tools archive ships its own NOTICE.txt. Redistributing adb.exe
# means redistributing that notice too (see LICENSES/ADB-NOTICE.txt).
$platformToolsNotice = Join-Path $depsDir 'PLATFORM_TOOLS_NOTICE.txt'
if (Test-Path $platformToolsNotice) {
    Copy-Item -Path $platformToolsNotice -Destination (Join-Path $licenseTarget 'PLATFORM_TOOLS_NOTICE.txt') -Force
}
else {
    Write-Warning 'deps/PLATFORM_TOOLS_NOTICE.txt is missing; the package would redistribute adb.exe without its upstream notice.'
}

# ---------------------------------------------------------------- package

if (Test-Path $packageDir) {
    Remove-Item -Path $packageDir -Recurse -Force
}

New-Item -ItemType Directory -Path $packageDir -Force | Out-Null
Copy-Item -Path (Join-Path $publishDir '*') -Destination $packageDir -Recurse -Force

if ($SkipZip) {
    Write-Host "Package directory: $packageDir" -ForegroundColor Green
    return
}

if (Test-Path $zipPath) {
    Remove-Item -Path $zipPath -Force
}

Write-Host 'Creating zip...' -ForegroundColor Cyan

# Entries are written by hand rather than with CreateFromDirectory: under
# Windows PowerShell 5.1 that API emits '\' separators, which violates the ZIP
# specification (APPNOTE 4.4.17.1) and makes strict extractors on other
# platforms create files literally named "LICENSES\ADB-NOTICE.txt".
Add-Type -AssemblyName System.IO.Compression.FileSystem

$sourceRoot = (Resolve-Path -LiteralPath $packageDir).Path.TrimEnd('\', '/')

$archive = [System.IO.Compression.ZipFile]::Open($zipPath, 'Create')
try {
    Get-ChildItem -LiteralPath $sourceRoot -Recurse -File | ForEach-Object {
        $relative = $_.FullName.Substring($sourceRoot.Length + 1) -replace '\\', '/'
        [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive,
            $_.FullName,
            $relative,
            [System.IO.Compression.CompressionLevel]::Optimal)
    }
}
finally {
    $archive.Dispose()
}

Write-Host "Package ready: $zipPath" -ForegroundColor Green
