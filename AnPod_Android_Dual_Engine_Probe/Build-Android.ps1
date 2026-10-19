# Run in PowerShell or CMD: powershell -NoProfile -ExecutionPolicy Bypass -File .\Build-Android.ps1
param(
    [string]$WrapperJarPath = '',
    [string]$Adb = '',
    [switch]$Install
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $root
$jar = Join-Path $root 'gradle\wrapper\gradle-wrapper.jar'

if (-not (Test-Path -LiteralPath $jar)) {
    if ($WrapperJarPath) {
        if (-not (Test-Path -LiteralPath $WrapperJarPath)) { throw "Cannot find wrapper jar: $WrapperJarPath" }
        Copy-Item -LiteralPath $WrapperJarPath -Destination $jar
    } else {
        Write-Host 'Gradle Wrapper jar not bundled. Downloading official Gradle v8.9.0 wrapper (no other files changed)...'
        $url = 'https://raw.githubusercontent.com/gradle/gradle/v8.9.0/gradle/wrapper/gradle-wrapper.jar'
        try {
            Invoke-WebRequest -Uri $url -OutFile $jar -UseBasicParsing
        } catch {
            Remove-Item -LiteralPath $jar -ErrorAction SilentlyContinue
            throw ('Could not obtain Gradle wrapper jar. Use -WrapperJarPath with your existing AnPod\src\AnPod.Receiver\gradle\wrapper\gradle-wrapper.jar. Detail: ' + $_.Exception.Message)
        }
    }
}
# ZIP signature sanity check; Gradle distribution itself is checksum pinned in properties.
$fh = [System.IO.File]::OpenRead($jar)
try {
    $magic = New-Object byte[] 2
    [void]$fh.Read($magic, 0, 2)
    if ($magic[0] -ne 0x50 -or $magic[1] -ne 0x4B) { throw 'Invalid Gradle wrapper jar (not a ZIP/JAR).' }
} finally { $fh.Dispose() }

Write-Host 'Building Debug APK (Android SDK/NDK/CMake and online Gradle/Maven dependencies required)...'
& (Join-Path $root 'gradlew.bat') ':app:assembleDebug' '--no-daemon'
if ($LASTEXITCODE -ne 0) { throw "Gradle build failed (exit=$LASTEXITCODE)" }
$apk = Join-Path $root 'app\build\outputs\apk\debug\app-debug.apk'
if (-not (Test-Path -LiteralPath $apk)) { throw "Build returned success but APK not found: $apk" }
Write-Host "BUILD SUCCESS: $apk"
if ($Install) {
    if (-not $Adb) {
        $found = Get-Command adb -ErrorAction SilentlyContinue
        if (-not $found) { throw 'ADB not found. Use -Adb "F:\Download\platform-tools-latest-windows\platform-tools\adb.exe"' }
        $Adb = $found.Source
    }
    if (-not (Test-Path -LiteralPath $Adb)) { throw "ADB not found: $Adb" }
    & $Adb start-server
    & $Adb install -r $apk
    if ($LASTEXITCODE -ne 0) { throw "adb install failed ($LASTEXITCODE)" }
    Write-Host 'Installed standalone app.anpod.audioprobe. Existing AnPod receiver untouched.'
}
