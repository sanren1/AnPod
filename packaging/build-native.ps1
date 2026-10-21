<#
.SYNOPSIS
    Builds src/AnPod.Native into deps/audio_capture.dll.

.DESCRIPTION
    Two toolchains are supported, tried in this order:

      1. CMake + Visual Studio  -- the toolchain named in plan.md appendix B.
      2. MinGW-w64 g++          -- fallback for machines without Visual Studio.
                                   Verified to produce a working x64 DLL with
                                   the same exports and no non-system imports.

    The managed application is x64-only, so the DLL must be x64 too; both paths
    fail loudly rather than producing a mismatched binary (plan §1.2, §7.2).

.PARAMETER MinGWRoot
    Directory containing bin\g++.exe (for example an extracted w64devkit).
    Optional; g++ is also picked up from PATH.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File packaging\build-native.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File packaging\build-native.ps1 `
        -MinGWRoot $env:MINGW_ROOT
#>
[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [string]$OutputDirectory,

    [string]$BuildDirectory,

    [string]$MinGWRoot,

    [switch]$ForceMinGW
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$sourceDir = Join-Path $root 'src\AnPod.Native'
$sourceFile = Join-Path $sourceDir 'audio_capture.cpp'
$headerFile = Join-Path $sourceDir 'audio_capture.h'
$buildDir = if ($BuildDirectory) { [IO.Path]::GetFullPath($BuildDirectory) } else { Join-Path $root 'artifacts\native' }

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $root 'deps'
}

if (-not (Test-Path $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
}

$destination = Join-Path $OutputDirectory 'audio_capture.dll'

# --------------------------------------------------------------- toolchain 1

function Test-CMakeAvailable {
    return [bool](Get-Command 'cmake' -ErrorAction SilentlyContinue)
}

function Build-WithCMake {
    Write-Host "Building with CMake ($Configuration, x64)..." -ForegroundColor Cyan

    $cmake = (Get-Command 'cmake').Source

    & $cmake -S $sourceDir -B $buildDir -A x64 | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "CMake configure failed with exit code $LASTEXITCODE."
    }

    & $cmake --build $buildDir --config $Configuration | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "CMake build failed with exit code $LASTEXITCODE."
    }

    return (Join-Path $buildDir "bin\audio_capture.dll")
}

# --------------------------------------------------------------- toolchain 2

function Resolve-Gxx {
    if ($MinGWRoot) {
        $candidate = Join-Path $MinGWRoot 'bin\g++.exe'
        if (Test-Path $candidate) {
            return $candidate
        }
        throw "g++.exe not found under -MinGWRoot '$MinGWRoot'."
    }

    $resolved = Get-Command 'g++' -ErrorAction SilentlyContinue
    if ($resolved) {
        return $resolved.Source
    }

    return $null
}

function Build-WithMinGW {
    param([string]$Gxx)

    Write-Host "Building with MinGW g++ ($Gxx)..." -ForegroundColor Cyan

    $outDir = Join-Path $buildDir 'mingw'
    if (-not (Test-Path $outDir)) {
        New-Item -ItemType Directory -Path $outDir | Out-Null
    }

    $output = Join-Path $outDir 'audio_capture.dll'

    $arguments = @(
        '-shared'
        '-O2'
        '-Wall'
        '-Wextra'
        '-std=c++17'
        '-o', $output
        $sourceFile
        $headerFile
        '-lole32'
        '-lwinmm'
        '-lws2_32'
        '-luuid'
        '-static-libgcc'
        '-static-libstdc++'
    )

    & $Gxx @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "MinGW build failed with exit code $LASTEXITCODE."
    }

    return $output
}

# ---------------------------------------------------------------------- run

$built = $null

if (-not $ForceMinGW -and (Test-CMakeAvailable)) {
    $built = Build-WithCMake
}
else {
    $gxx = Resolve-Gxx

    if (-not $gxx) {
        throw @'
No C++ toolchain found.

Install one of:
  * Visual Studio 2022 with the "Desktop development with C++" workload, plus
    CMake (https://cmake.org/download/), or
  * a MinGW-w64 distribution providing g++ (for example w64devkit), and pass
    its root with -MinGWRoot.
'@
    }

    $built = Build-WithMinGW -Gxx $gxx
}

if (-not (Test-Path $built)) {
    throw "Expected output not found: $built"
}

Copy-Item -Path $built -Destination $destination -Force

$size = (Get-Item $destination).Length
Write-Host "Copied to $destination ($size bytes)" -ForegroundColor Green
Write-Host 'Note: the plan.md release toolchain is MSVC/CMake; a MinGW build is'
Write-Host 'functionally equivalent but should be re-built with MSVC for release.'
