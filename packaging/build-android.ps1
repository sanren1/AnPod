<#
.SYNOPSIS
    Builds src/AnPod.AndroidServer into deps/server.

.DESCRIPTION
    The "server" payload is a zip containing classes.dex. It is pushed to
    /data/local/tmp and started with app_process; it is never installed.

    Two build paths are supported:

      1. Gradle + Android SDK  -- produces an APK, which is an equally valid
                                  classes.dex container.
      2. SDK direct            -- javac + d8 + zip, no Gradle. Fallback for
                                  machines without Gradle, and much faster.

    Both paths emit the same file: deps/server.

.PARAMETER AndroidSdkRoot
    An Android SDK (or a directory containing platforms\ and build-tools\).
    Defaults to %LOCALAPPDATA%\Android\Sdk.

.PARAMETER JavaHome
    JDK root containing bin\javac.exe. Defaults to $env:JAVA_HOME, then PATH.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File packaging\build-android.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File packaging\build-android.ps1 `
        -AndroidSdkRoot $env:ANDROID_SDK_ROOT -JavaHome $env:JAVA_HOME
#>
[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Variant = 'debug',

    [string]$OutputDirectory,

    [string]$BuildDirectory,

    [string]$AndroidSdkRoot,

    [string]$JavaHome,

    [switch]$ForceSdkDirect
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$projectDir = Join-Path $root 'src\AnPod.AndroidServer'
$javaSourceDir = Join-Path $projectDir 'src\main\java'
$buildDir = if ($BuildDirectory) { [IO.Path]::GetFullPath($BuildDirectory) } else { Join-Path $root 'artifacts\android' }

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $root 'deps'
}

if (-not (Test-Path $OutputDirectory)) {
    New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
}

$payloadPath = Join-Path $OutputDirectory 'server'

# ------------------------------------------------------------------- helpers

function Resolve-JavaHome {
    if ($JavaHome) {
        if (Test-Path (Join-Path $JavaHome 'bin\javac.exe')) {
            return $JavaHome
        }
        throw "javac.exe not found under -JavaHome '$JavaHome'."
    }

    if ($env:JAVA_HOME -and (Test-Path (Join-Path $env:JAVA_HOME 'bin\javac.exe'))) {
        return $env:JAVA_HOME
    }

    $javac = Get-Command 'javac' -ErrorAction SilentlyContinue
    if ($javac) {
        return (Split-Path -Parent (Split-Path -Parent $javac.Source))
    }

    return $null
}

function Resolve-AndroidSdkRoot {
    if ($AndroidSdkRoot) {
        return $AndroidSdkRoot
    }

    $default = Join-Path $env:LOCALAPPDATA 'Android\Sdk'
    if (Test-Path $default) {
        return $default
    }

    return $null
}

function Find-AndroidJar {
    param([string]$SdkRoot)

    if (-not $SdkRoot -or -not (Test-Path (Join-Path $SdkRoot 'platforms'))) {
        return $null
    }

    # Prefer the newest android-XX platform available.
    $jar = Get-ChildItem -Path (Join-Path $SdkRoot 'platforms') -Directory |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'android.jar' } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1

    return $jar
}

function Find-D8 {
    param([string]$SdkRoot)

    if (-not $SdkRoot -or -not (Test-Path (Join-Path $SdkRoot 'build-tools'))) {
        return $null
    }

    return Get-ChildItem -Path (Join-Path $SdkRoot 'build-tools') -Directory |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'd8.bat' } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1
}

# --------------------------------------------------------------- toolchain 1

function Test-GradleAvailable {
    if (Test-Path (Join-Path $projectDir 'gradlew.bat')) {
        return $true
    }

    return [bool](Get-Command 'gradle' -ErrorAction SilentlyContinue)
}

function Build-WithGradle {
    $gradleCommand = if (Test-Path (Join-Path $projectDir 'gradlew.bat')) {
        Join-Path $projectDir 'gradlew.bat'
    }
    else {
        (Get-Command 'gradle').Source
    }

    $task = "assemble$($Variant.Substring(0, 1).ToUpper())$($Variant.Substring(1))"

    Write-Host "Building with Gradle (task: $task)..." -ForegroundColor Cyan

    Push-Location $projectDir
    try {
        & $gradleCommand $task "-PanpodPayloadPath=$payloadPath" "-PanpodBuildDir=$buildDir" --no-daemon
        if ($LASTEXITCODE -ne 0) {
            throw "Gradle failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
}

# --------------------------------------------------------------- toolchain 2

function Build-WithSdkDirect {
    param(
        [string]$JdkRoot,
        [string]$AndroidJar,
        [string]$D8
    )

    Write-Host "Building with javac + d8 (SDK direct)..." -ForegroundColor Cyan

    $classesDir = Join-Path $buildDir 'classes'
    $dexDir = Join-Path $buildDir 'dex'

    foreach ($dir in @($classesDir, $dexDir)) {
        if (Test-Path $dir) {
            Remove-Item -Path $dir -Recurse -Force
        }
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }

    $javac = Join-Path $JdkRoot 'bin\javac.exe'
    $protocolSourceDir = Join-Path $root 'src\AnPod.AndroidProtocol\src\main\java'
    $sources = @(
        Get-ChildItem -Path $javaSourceDir -Recurse -Filter '*.java' | ForEach-Object { $_.FullName }
        Get-ChildItem -Path $protocolSourceDir -Recurse -Filter '*.java' | ForEach-Object { $_.FullName }
    )

    if (-not $sources) {
        throw "No Java sources found under $javaSourceDir."
    }

    # --release 8 keeps JDK 9+ APIs out of the bytecode; android.jar supplies
    # the android.* and java.* surface the payload is allowed to use.
    & $javac --release 8 -encoding UTF-8 -classpath $AndroidJar -d $classesDir @sources
    if ($LASTEXITCODE -ne 0) {
        throw "javac failed with exit code $LASTEXITCODE."
    }

    $classFiles = Get-ChildItem -Path $classesDir -Recurse -Filter '*.class' |
        ForEach-Object { $_.FullName }

    if (-not $classFiles) {
        throw 'javac produced no class files.'
    }

    # d8.bat needs a JVM; point it at the JDK we just used.
    $previousJavaHome = $env:JAVA_HOME
    $env:JAVA_HOME = $JdkRoot

    try {
        & $D8 --lib $AndroidJar --min-api 23 --output $dexDir @classFiles
        if ($LASTEXITCODE -ne 0) {
            throw "d8 failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        $env:JAVA_HOME = $previousJavaHome
    }

    $dex = Join-Path $dexDir 'classes.dex'
    if (-not (Test-Path $dex)) {
        throw "d8 did not produce classes.dex (looked in $dexDir)."
    }

    # Build the container zip explicitly so the entry is named exactly
    # 'classes.dex' at the root - Compress-Archive is not guaranteed to.
    if (Test-Path $payloadPath) {
        Remove-Item -Path $payloadPath -Force
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $archive = [System.IO.Compression.ZipFile]::Open($payloadPath, 'Create')
    try {
        [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive,
            $dex,
            'classes.dex',
            [System.IO.Compression.CompressionLevel]::Optimal)
    }
    finally {
        $archive.Dispose()
    }

    Write-Host "classes.dex: $((Get-Item $dex).Length) bytes" -ForegroundColor DarkGray
}

# ---------------------------------------------------------------------- run

if (-not $ForceSdkDirect -and (Test-GradleAvailable)) {
    Build-WithGradle
}
else {
    $jdkRoot = Resolve-JavaHome
    if (-not $jdkRoot) {
        throw @'
No JDK found.

Set JAVA_HOME, pass -JavaHome, or install a JDK 17 (which provides javac).
A JRE alone is not sufficient.
'@
    }

    $sdkRoot = Resolve-AndroidSdkRoot
    $androidJar = Find-AndroidJar -SdkRoot $sdkRoot
    $d8 = Find-D8 -SdkRoot $sdkRoot

    if (-not $androidJar -or -not $d8) {
        throw @'
Android SDK components are missing.

Need:
  * <SdkRoot>\platforms\android-XX\android.jar
  * <SdkRoot>\build-tools\<ver>\d8.bat

Pass -AndroidSdkRoot, or install the SDK via Android Studio / sdkmanager.
'@
    }

    Write-Host "  java home   : $jdkRoot"
    Write-Host "  android.jar : $androidJar"
    Write-Host "  d8          : $d8"

    Build-WithSdkDirect -JdkRoot $jdkRoot -AndroidJar $androidJar -D8 $d8
}

if (-not (Test-Path $payloadPath)) {
    throw "Expected payload not found: $payloadPath"
}

$size = (Get-Item $payloadPath).Length
Write-Host "Payload ready: $payloadPath ($size bytes)" -ForegroundColor Green
Write-Host 'Verify it contains classes.dex at the root before shipping.'
