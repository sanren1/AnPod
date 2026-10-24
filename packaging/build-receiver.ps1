[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Variant = 'debug',
    [string]$JavaHome,
    [string]$AndroidSdkRoot,
    [string]$OutputDirectory,
    [string]$DebugKeyStorePath,
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'source-snapshot.ps1')
$inputPaths = @('src/AnPod.Receiver', 'src/AnPod.AndroidProtocol', 'packaging/build-receiver.ps1', 'packaging/source-snapshot.ps1')
$sourceInputsSha256 = Get-AnPodSourceSnapshot $root $inputPaths
$project = Join-Path $root 'src\AnPod.Receiver'

if (-not $JavaHome) { $JavaHome = $env:JAVA_HOME }
if (-not $JavaHome -or -not (Test-Path -LiteralPath (Join-Path $JavaHome 'bin\java.exe'))) {
    throw 'JDK 17 is required. Pass -JavaHome or set JAVA_HOME.'
}
if (-not $AndroidSdkRoot) { $AndroidSdkRoot = $env:ANDROID_SDK_ROOT }
if (-not $AndroidSdkRoot) { $AndroidSdkRoot = $env:ANDROID_HOME }
if (-not $AndroidSdkRoot -or -not (Test-Path -LiteralPath (Join-Path $AndroidSdkRoot 'platforms\android-35\android.jar'))) {
    throw 'Android SDK Platform 35 is required. Pass -AndroidSdkRoot or set ANDROID_SDK_ROOT.'
}
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $root 'artifacts\p8-v2-receiver' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$appBuildDirectory = Join-Path $OutputDirectory 'gradle-app-build'
if ($DebugKeyStorePath) {
    $DebugKeyStorePath = [IO.Path]::GetFullPath($DebugKeyStorePath)
    if ($Variant -ne 'debug') { throw '-DebugKeyStorePath is only valid for the debug candidate variant.' }
    if (-not (Test-Path -LiteralPath $DebugKeyStorePath -PathType Leaf)) { throw 'The selected local candidate keystore was not found.' }
}
if ((Test-Path -LiteralPath (Join-Path $OutputDirectory 'receiver.apk')) -or
        (Test-Path -LiteralPath (Join-Path $OutputDirectory 'receiver.json'))) {
    throw "Refusing to overwrite an existing Receiver artifact in $OutputDirectory. Choose a fresh isolated output directory."
}

$appGradle = Get-Content -LiteralPath (Join-Path $project 'app\build.gradle') -Raw
$applicationId = [regex]::Match($appGradle, "(?m)^\s*applicationId\s+'([^']+)'\s*$").Groups[1].Value
$configuredVersionCode = [int][regex]::Match($appGradle, '(?m)^\s*versionCode\s+(\d+)\s*$').Groups[1].Value
$configuredVersionName = [regex]::Match($appGradle, "(?m)^\s*versionName\s+'([^']+)'\s*$").Groups[1].Value
$configuredTargetSdk = [int][regex]::Match($appGradle, '(?m)^\s*targetSdk\s+(\d+)\s*$').Groups[1].Value
if ($applicationId -ne 'app.anpod.receiver' -or $configuredVersionCode -le 0 -or $configuredVersionName -notmatch '^\d+\.\d+\.\d+-v3-flow-candidate$' -or $configuredTargetSdk -ne 35) {
    throw 'Receiver Gradle configuration must identify app.anpod.receiver with a positive versionCode, semantic v3 flow candidate versionName and targetSdk 35.'
}
$sourceCommit = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $sourceCommit -notmatch '^[0-9a-f]{40}$') { throw 'Could not resolve the Receiver source commit.' }

if ($Variant -eq 'release') {
    $requiredSigningVariables = @(
        'ANPOD_RECEIVER_RELEASE_STORE_FILE',
        'ANPOD_RECEIVER_RELEASE_STORE_PASSWORD',
        'ANPOD_RECEIVER_RELEASE_KEY_ALIAS',
        'ANPOD_RECEIVER_RELEASE_KEY_PASSWORD'
    )
    $missingSigningVariables = @($requiredSigningVariables | Where-Object { [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($_)) })
    if ($missingSigningVariables.Count) {
        throw 'Release signing needs all ANPOD_RECEIVER_RELEASE_* values from a private environment. No key is generated or written by this script.'
    }
    if (-not (Test-Path -LiteralPath $env:ANPOD_RECEIVER_RELEASE_STORE_FILE -PathType Leaf)) {
        throw 'The configured private Receiver release keystore was not found.'
    }
}

$buildTools = Get-ChildItem -LiteralPath (Join-Path $AndroidSdkRoot 'build-tools') -Directory |
    Where-Object { (Test-Path -LiteralPath (Join-Path $_.FullName 'aapt.exe')) -and (Test-Path -LiteralPath (Join-Path $_.FullName 'apksigner.bat')) } |
    Sort-Object { [version]((Get-Content -LiteralPath (Join-Path $_.FullName 'source.properties') | Where-Object { $_ -like 'Pkg.Revision=*' } | Select-Object -First 1) -replace '^Pkg.Revision=', '') } -Descending |
    Select-Object -First 1
if (-not $buildTools) { throw 'Android SDK Build-Tools with aapt.exe and apksigner.bat are required.' }

$oldJavaHome = $env:JAVA_HOME
$oldAndroidHome = $env:ANDROID_HOME
$oldAndroidSdkRoot = $env:ANDROID_SDK_ROOT
$oldCandidateDebugKeyStore = $env:ANPOD_RECEIVER_DEBUG_KEYSTORE
$oldPath = $env:PATH
$env:JAVA_HOME = [IO.Path]::GetFullPath($JavaHome)
$env:ANDROID_HOME = [IO.Path]::GetFullPath($AndroidSdkRoot)
$env:ANDROID_SDK_ROOT = $env:ANDROID_HOME
$env:PATH = (Join-Path $env:JAVA_HOME 'bin') + [IO.Path]::PathSeparator + $env:PATH
if ($DebugKeyStorePath) { $env:ANPOD_RECEIVER_DEBUG_KEYSTORE = $DebugKeyStorePath }

$task = ":app:assemble$($Variant.Substring(0, 1).ToUpperInvariant())$($Variant.Substring(1))"
$gradleArgs = @('--no-daemon', '--console', 'plain')
if ($Offline) { $gradleArgs += '--offline' }
$gradleArgs += $task
$gradleArgs += "-PreceiverBuildDir=$appBuildDirectory"

Push-Location $project
try {
    & (Join-Path $project 'gradlew.bat') @gradleArgs
    if ($LASTEXITCODE -ne 0) { throw "Receiver Gradle build failed with exit code $LASTEXITCODE." }
}
finally {
    Pop-Location
    $env:JAVA_HOME = $oldJavaHome
    $env:ANDROID_HOME = $oldAndroidHome
    $env:ANDROID_SDK_ROOT = $oldAndroidSdkRoot
    $env:ANPOD_RECEIVER_DEBUG_KEYSTORE = $oldCandidateDebugKeyStore
    $env:PATH = $oldPath
}

$apk = Join-Path $appBuildDirectory "outputs\apk\$Variant\app-$Variant.apk"
if (-not (Test-Path -LiteralPath $apk -PathType Leaf)) { throw "Gradle did not create the expected APK: $apk" }

$mergedManifest = Get-ChildItem -LiteralPath (Join-Path $appBuildDirectory "intermediates\merged_manifest\$Variant") -Recurse -Filter 'AndroidManifest.xml' -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $mergedManifest) { throw 'Gradle did not produce a merged manifest to audit.' }

[xml]$manifest = Get-Content -LiteralPath $mergedManifest.FullName -Raw
$androidNs = 'http://schemas.android.com/apk/res/android'
$ns = [Xml.XmlNamespaceManager]::new($manifest.NameTable)
$ns.AddNamespace('android', $androidNs)
$application = $manifest.SelectSingleNode('/manifest/application', $ns)
if (-not $application) { throw 'Merged manifest is missing application.' }
$components = @($application.ChildNodes | Where-Object { $_.LocalName -in @('activity', 'service', 'provider', 'receiver') })
$exports = @($components | Where-Object { $_.GetAttribute('exported', $androidNs) -eq 'true' })
$mainActivity = $components | Where-Object { $_.LocalName -eq 'activity' -and $_.GetAttribute('name', $androidNs) -eq 'app.anpod.receiver.MainActivity' }
$bootstrap = $components | Where-Object { $_.LocalName -eq 'activity' -and $_.GetAttribute('name', $androidNs) -eq 'app.anpod.receiver.BootstrapActivity' }
$service = $components | Where-Object { $_.LocalName -eq 'service' -and $_.GetAttribute('name', $androidNs) -eq 'app.anpod.receiver.ReceiverService' }
$provider = $components | Where-Object { $_.LocalName -eq 'provider' -and $_.GetAttribute('authorities', $androidNs) -eq 'app.anpod.receiver.state' }
$profileInstaller = $components | Where-Object { $_.LocalName -eq 'receiver' -and $_.GetAttribute('name', $androidNs) -eq 'androidx.profileinstaller.ProfileInstallReceiver' }
if (-not $bootstrap -or $bootstrap.GetAttribute('exported', $androidNs) -ne 'true' -or $bootstrap.GetAttribute('permission', $androidNs) -ne 'android.permission.DUMP') { throw 'Bootstrap must be explicitly exported and protected by android.permission.DUMP.' }
if (-not $mainActivity -or $mainActivity.GetAttribute('exported', $androidNs) -ne 'true' -or $mainActivity.GetAttribute('permission', $androidNs)) { throw 'MainActivity must be the unpermissioned exported Launcher entry.' }
if (-not $service -or $service.GetAttribute('exported', $androidNs) -ne 'false' -or
        ((@($service.GetAttribute('foregroundServiceType', $androidNs).Split('|') | Sort-Object) -join '|') -ne 'connectedDevice|mediaPlayback|shortService')) {
    throw 'Receiver service must remain private and declare exactly connectedDevice, mediaPlayback and shortService foreground types.'
}
if (-not $provider -or $provider.GetAttribute('exported', $androidNs) -ne 'true' -or $provider.GetAttribute('readPermission', $androidNs) -ne 'android.permission.DUMP' -or $provider.GetAttribute('writePermission', $androidNs) -ne 'android.permission.DUMP' -or $provider.GetAttribute('grantUriPermissions', $androidNs) -ne 'false') { throw 'Health provider must be read-only to ordinary callers and protected by android.permission.DUMP.' }
if (-not $profileInstaller -or $profileInstaller.GetAttribute('exported', $androidNs) -ne 'true' -or $profileInstaller.GetAttribute('permission', $androidNs) -ne 'android.permission.DUMP') { throw 'AndroidX ProfileInstallReceiver must remain protected by android.permission.DUMP.' }
if (@($exports | Where-Object { $_ -ne $bootstrap -and $_ -ne $provider -and $_ -ne $mainActivity -and $_ -ne $profileInstaller }).Count) { throw 'Merged manifest contains an unexpected exported component.' }
$launcherComponents = @($components | Where-Object { $_.SelectNodes('intent-filter[action[@android:name="android.intent.action.MAIN"] and category[@android:name="android.intent.category.LAUNCHER"]]', $ns).Count -gt 0 })
if ($launcherComponents.Count -ne 1 -or $launcherComponents[0] -ne $mainActivity) { throw 'MainActivity must be the only MAIN/LAUNCHER component.' }
$permissions = @($manifest.SelectNodes('/manifest/uses-permission', $ns) | ForEach-Object { $_.GetAttribute('name', $androidNs) })
foreach ($required in @('android.permission.INTERNET', 'android.permission.ACCESS_NETWORK_STATE', 'android.permission.ACCESS_WIFI_STATE', 'android.permission.WAKE_LOCK', 'android.permission.CHANGE_NETWORK_STATE', 'android.permission.FOREGROUND_SERVICE', 'android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK', 'android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE', 'android.permission.POST_NOTIFICATIONS')) {
    if ($required -notin $permissions) { throw "Merged manifest is missing $required." }
}
foreach ($retired in @('android.permission.FOREGROUND_SERVICE_DATA_SYNC', 'android.permission.REQUEST_INSTALL_PACKAGES')) {
    if ($retired -in $permissions) { throw "Retired network APK installation permission remains: $retired" }
}
if ('android.permission.ACCESS_LOCAL_NETWORK' -in $permissions) { throw 'Do not add a new local-network permission while targeting SDK 35.' }

$badging = & (Join-Path $buildTools.FullName 'aapt.exe') dump badging $apk
if ($LASTEXITCODE -ne 0) { throw 'aapt could not inspect the Receiver APK.' }
$packageLine = $badging | Where-Object { $_ -match '^package:' } | Select-Object -First 1
if ($packageLine -notmatch "name='$([regex]::Escape($applicationId))' versionCode='(?<code>\d+)' versionName='(?<name>[^']+)'") { throw 'APK package identity or version does not match the current Receiver Gradle configuration.' }
$versionCode = [int]$Matches.code
$versionName = $Matches.name
if (($badging -join "`n") -notmatch "targetSdkVersion:'$configuredTargetSdk'") { throw 'Receiver APK target SDK does not match the current Gradle configuration.' }

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($apk)
try {
    if (-not $archive.GetEntry('AndroidManifest.xml') -or -not $archive.GetEntry('classes.dex')) { throw 'APK is missing AndroidManifest.xml or classes.dex.' }
}
finally { $archive.Dispose() }

$signingJavaHome = $env:JAVA_HOME
$signingPath = $env:PATH
$env:JAVA_HOME = [IO.Path]::GetFullPath($JavaHome)
$env:PATH = (Join-Path $env:JAVA_HOME 'bin') + [IO.Path]::PathSeparator + $signingPath
try { $signatureOutput = & (Join-Path $buildTools.FullName 'apksigner.bat') verify --print-certs $apk 2>&1 }
finally { $env:JAVA_HOME = $signingJavaHome; $env:PATH = $signingPath }
$signatureText = $signatureOutput -join "`n"
$signerMatch = [regex]::Match($signatureText, 'Signer #1 certificate SHA-256 digest: (?<sha>[0-9a-fA-F]{64})')
if ($LASTEXITCODE -ne 0 -or -not $signerMatch.Success) {
    throw "APK signature verification failed: $($signatureOutput -join ' ')"
}
$signerSha256 = $signerMatch.Groups['sha'].Value.ToLowerInvariant()
$apkSha256 = (Get-FileHash -LiteralPath $apk -Algorithm SHA256).Hash.ToLowerInvariant()
if ($versionName -ne $configuredVersionName -or $versionCode -ne $configuredVersionCode) {
    throw 'Receiver APK version does not match the current v3 candidate Gradle configuration.'
}

$outputPath = $OutputDirectory
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null
$apkDestination = Join-Path $outputPath 'receiver.apk'
$metadataDestination = Join-Path $outputPath 'receiver.json'
$temporaryApk = "$apkDestination.new"
$temporaryMetadata = "$metadataDestination.new"
Copy-Item -LiteralPath $apk -Destination $temporaryApk -Force
$metadata = [ordered]@{
    applicationId = $applicationId
    versionCode = $versionCode
    versionName = $versionName
    variant = $Variant
    targetSdk = $configuredTargetSdk
    sha256 = $apkSha256
    signerSha256 = $signerSha256
    sourceCommit = $sourceCommit
    sourceInputsSha256 = $sourceInputsSha256
    sourceModified = (Test-AnPodSourceModified -Root $root -Paths $inputPaths)
}
if ((Get-AnPodSourceSnapshot $root $inputPaths) -ne $sourceInputsSha256) { throw 'Receiver source inputs changed during the build.' }
$metadataJson = $metadata | ConvertTo-Json
$utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($temporaryMetadata, $metadataJson, $utf8WithoutBom)
$metadataBytes = [System.IO.File]::ReadAllBytes($temporaryMetadata)
if ($metadataBytes.Length -ge 3 -and $metadataBytes[0] -eq 0xEF -and $metadataBytes[1] -eq 0xBB -and $metadataBytes[2] -eq 0xBF) {
    throw 'Receiver metadata must be UTF-8 without a BOM so Rust serde_json can read it.'
}
$decodedMetadata = [System.Text.UTF8Encoding]::new($false, $true).GetString($metadataBytes)
$null = $decodedMetadata | ConvertFrom-Json
Move-Item -LiteralPath $temporaryApk -Destination $apkDestination -Force
Move-Item -LiteralPath $temporaryMetadata -Destination $metadataDestination -Force

Write-Host "Receiver APK: $apkDestination ($Variant, version $versionName / $versionCode)" -ForegroundColor Green
Write-Host "SHA-256: $apkSha256"
Write-Host "Signer certificate SHA-256: $signerSha256"
