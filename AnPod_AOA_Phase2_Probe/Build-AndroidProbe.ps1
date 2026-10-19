# Builds the standalone Android APK using an existing Gradle wrapper from your AnPod source tree.
# It NEVER overwrites AnPod Receiver or modifies the installed Android application.
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$GradlewPath)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $GradlewPath -PathType Leaf)) {
    throw "Existing gradlew.bat not found: $GradlewPath"
}
$project = Join-Path $PSScriptRoot 'AndroidProbe'
$gradlew = (Resolve-Path -LiteralPath $GradlewPath).Path
Push-Location $project
try {
    & $gradlew --no-daemon -p $project :app:assembleDebug
    if ($LASTEXITCODE -ne 0) { throw "Gradle exit code $LASTEXITCODE" }
    $apk = Join-Path $project 'app\build\outputs\apk\debug\app-debug.apk'
    if (-not (Test-Path -LiteralPath $apk)) { throw "APK missing: $apk" }
    Write-Host "BUILD OK: $apk" -ForegroundColor Green
    Write-Host 'Install this standalone APK using adb install -r <path>. Existing AnPod Receiver remains unchanged.'
} finally { Pop-Location }
