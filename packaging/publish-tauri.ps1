[CmdletBinding()]
param(
    [string]$Version,
    [switch]$IncludeReceiverCandidate,
    [switch]$IncludeReceiverRelease,
    [ValidateSet('p1', 'p2', 'v2', 'v3')]
    [string]$ReceiverCandidate = 'p1',
    [string]$ReceiverArtifactDirectory,
    [string]$NativeArtifactDirectory,
    [string]$LegacyPayloadDirectory,
    [string]$RustTargetDirectory,
    [string]$AndroidSdkRoot,
    [switch]$Offline,
    [switch]$ReviewPackage,
    [switch]$ReuseExistingBuild,
    [switch]$ReviewWorkingTree
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$config = Get-Content -Raw (Join-Path $root 'src-tauri\tauri.conf.json') | ConvertFrom-Json
if (-not $Version) { $Version = $config.version }
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw "Invalid package version: $Version" }
. (Join-Path $PSScriptRoot 'source-snapshot.ps1')
if ($ReviewWorkingTree -and (-not $ReviewPackage -or $ReceiverCandidate -ne 'v3')) { throw '-ReviewWorkingTree requires a local V3 review candidate.' }
$desktopInputPaths = @('src-tauri', 'frontend', 'packaging/publish-tauri.ps1', 'packaging/source-snapshot.ps1')
$desktopInputsSha256 = Get-AnPodSourceSnapshot $root $desktopInputPaths
$deps = Join-Path $root 'deps'
$artifacts = Join-Path $root 'artifacts'
if ($ReviewPackage -and (-not $IncludeReceiverCandidate -or $ReceiverCandidate -notin @('v2', 'v3'))) { throw '-ReviewPackage is only valid for the v2 or v3 Receiver candidate.' }
if ($ReuseExistingBuild -and (-not $IncludeReceiverCandidate -or $ReceiverCandidate -ne 'v2')) { throw '-ReuseExistingBuild is only valid for the v2 Receiver candidate.' }
if ($ReceiverCandidate -eq 'v3' -and -not $ReviewPackage) { throw 'V3 can only be packaged as a review candidate.' }
if ($IncludeReceiverRelease) {
    if ($IncludeReceiverCandidate) { throw 'Choose either -IncludeReceiverCandidate or -IncludeReceiverRelease, not both.' }
    if ($ReviewPackage -or $ReviewWorkingTree) { throw '-IncludeReceiverRelease cannot be combined with review switches; review packages keep the audited debug candidate.' }
    if (-not $ReceiverArtifactDirectory) { throw 'Release packaging requires -ReceiverArtifactDirectory built by build-receiver.ps1 -Variant release.' }
    $releaseReceiverSource = [IO.Path]::GetFullPath($(if ([IO.Path]::IsPathRooted($ReceiverArtifactDirectory)) { $ReceiverArtifactDirectory } else { Join-Path $root $ReceiverArtifactDirectory }))
    $releaseArtifactsPrefix = [IO.Path]::GetFullPath($artifacts).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    if (-not $releaseReceiverSource.StartsWith($releaseArtifactsPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Release Receiver artifacts must live under artifacts/: build them into a fresh isolated output directory.'
    }
    foreach ($releaseFile in @('receiver.apk', 'receiver.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $releaseReceiverSource $releaseFile) -PathType Leaf)) { throw "Release Receiver directory is missing $releaseFile." }
    }
}
$reviewSuffix = if ($ReviewPackage) { '-review' } else { '' }
$packageName = if ($IncludeReceiverCandidate -and $ReceiverCandidate -eq 'v3') {
    "AnPod-$Version-receiver-v3-usb-oneclick-review-tauri-win-x64-portable"
} elseif ($IncludeReceiverCandidate) {
    "AnPod-$Version-receiver-$ReceiverCandidate-candidate$reviewSuffix-tauri-win-x64-portable"
} else { "AnPod-$Version-tauri-win-x64-portable" }
$manifest = Get-Content -Raw (Join-Path $root 'src-tauri\Cargo.toml')
$config = Get-Content -Raw (Join-Path $root 'src-tauri\tauri.conf.json') | ConvertFrom-Json
$manifestVersion = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($manifestVersion -ne $Version -or $config.version -ne $Version) { throw "Requested $Version but Cargo/Tauri versions are $manifestVersion/$($config.version)." }

New-Item -ItemType Directory -Path $artifacts -Force | Out-Null
$artifactsRoot = [System.IO.Path]::GetFullPath($artifacts).TrimEnd('\', '/')
$packageDir = [System.IO.Path]::GetFullPath((Join-Path $artifacts $packageName))
$zipPath = [System.IO.Path]::GetFullPath((Join-Path $artifacts "$packageName.zip"))
foreach ($target in @($packageDir, $zipPath)) {
    if (-not $target.StartsWith($artifactsRoot + [System.IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to write outside artifacts/: $target"
    }
}
if ($IncludeReceiverCandidate -and $ReceiverCandidate -in @('v2', 'v3') -and ((Test-Path -LiteralPath $packageDir) -or (Test-Path -LiteralPath $zipPath))) {
    throw "Refusing to overwrite an existing v$ReceiverCandidate candidate path: $packageName"
}

if ($ReceiverCandidate -in @('p2', 'v2', 'v3') -and $IncludeReceiverCandidate) {
    if ($ReceiverCandidate -eq 'v2' -and -not $RustTargetDirectory) { throw 'V2 packaging requires an explicit isolated RustTargetDirectory.' }
    if ($ReceiverCandidate -eq 'v3' -and -not $RustTargetDirectory) { throw 'V3 packaging requires an explicit new isolated RustTargetDirectory.' }
    if (-not $RustTargetDirectory) { $RustTargetDirectory = 'artifacts\p2-tauri-target' }
    $targetDir = if ([IO.Path]::IsPathRooted($RustTargetDirectory)) { [IO.Path]::GetFullPath($RustTargetDirectory) } else { [IO.Path]::GetFullPath((Join-Path $root $RustTargetDirectory)) }
} else {
    if (-not $RustTargetDirectory) { $RustTargetDirectory = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'src-tauri\target' } }
    $targetDir = [System.IO.Path]::GetFullPath($RustTargetDirectory)
}

$npm = Get-Command npm -ErrorAction SilentlyContinue
$frontendTools = Join-Path $root 'frontend\node_modules\.bin'
$tsc = Join-Path $frontendTools 'tsc.cmd'
$vite = Join-Path $frontendTools 'vite.cmd'
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $ReuseExistingBuild) {
    if (-not $npm -and (-not (Test-Path -LiteralPath $tsc -PathType Leaf) -or -not (Test-Path -LiteralPath $vite -PathType Leaf))) { throw 'npm or the already-installed local TypeScript/Vite commands are required.' }
    if (-not $cargo) { throw 'cargo was not found on PATH. Install Rust stable 1.90 or newer.' }
} elseif (-not (Test-Path -LiteralPath (Join-Path $targetDir 'release\anpod-tauri.exe') -PathType Leaf)) {
    throw 'The selected isolated target does not contain the previously audited Tauri release executable.'
}

$required = @('adb.exe', 'AdbWinApi.dll', 'AdbWinUsbApi.dll', 'audio_capture.dll', 'server', 'PLATFORM_TOOLS_NOTICE.txt')
$receiverFiles = @('receiver.apk', 'receiver.json')
if ($NativeArtifactDirectory) {
    $nativeSource = if ([IO.Path]::IsPathRooted($NativeArtifactDirectory)) { [IO.Path]::GetFullPath($NativeArtifactDirectory) } else { [IO.Path]::GetFullPath((Join-Path $root $NativeArtifactDirectory)) }
} else { $nativeSource = $deps }
if ($LegacyPayloadDirectory) {
    $legacyPayloadSource = if ([IO.Path]::IsPathRooted($LegacyPayloadDirectory)) { [IO.Path]::GetFullPath($LegacyPayloadDirectory) } else { [IO.Path]::GetFullPath((Join-Path $root $LegacyPayloadDirectory)) }
} else { $legacyPayloadSource = $deps }
if ($ReceiverArtifactDirectory) {
    $receiverSource = if ([IO.Path]::IsPathRooted($ReceiverArtifactDirectory)) { [IO.Path]::GetFullPath($ReceiverArtifactDirectory) } else { [IO.Path]::GetFullPath((Join-Path $root $ReceiverArtifactDirectory)) }
} else { $receiverSource = $deps }
if ($IncludeReceiverCandidate) {
    if ($ReceiverCandidate -eq 'p2' -and -not $ReceiverArtifactDirectory) { throw 'P2 packaging requires an explicit isolated ReceiverArtifactDirectory.' }
    if ($ReceiverCandidate -eq 'p2' -and [IO.Path]::GetFullPath($receiverSource).Equals([IO.Path]::GetFullPath($deps), [StringComparison]::OrdinalIgnoreCase)) { throw 'P2 packaging must not use deps/; preserve the existing P1 artifacts.' }
    if ($ReceiverCandidate -eq 'p2' -and -not $NativeArtifactDirectory) { throw 'P2 packaging requires an explicitly built isolated NativeArtifactDirectory.' }
    if ($ReceiverCandidate -eq 'p2' -and $nativeSource.Equals([IO.Path]::GetFullPath($deps), [StringComparison]::OrdinalIgnoreCase)) { throw 'P2 packaging must not use deps/audio_capture.dll.' }
    if ($ReceiverCandidate -eq 'p2' -and -not $LegacyPayloadDirectory) { throw 'P2 packaging requires an isolated legacy audio payload build.' }
    if ($ReceiverCandidate -eq 'p2' -and $legacyPayloadSource.Equals([IO.Path]::GetFullPath($deps), [StringComparison]::OrdinalIgnoreCase)) { throw 'P2 packaging must not use deps/server.' }
    $missingReceiver = @($receiverFiles | Where-Object { -not (Test-Path -LiteralPath (Join-Path $receiverSource $_) -PathType Leaf) })
    if ($missingReceiver.Count) { throw "Receiver candidate needs both files in the selected artifact directory: $($missingReceiver -join ', ')" }
    if ($ReceiverCandidate -eq 'p2') {
        $receiverMetadata = Get-Content -Raw (Join-Path $receiverSource 'receiver.json') | ConvertFrom-Json
        if ($receiverMetadata.versionName -ne '0.2.0-p2-candidate' -or $receiverMetadata.versionCode -ne 2) { throw 'P2 package requires a version 0.2.0-p2-candidate / code 2 Receiver artifact.' }
    }
    if ($ReceiverCandidate -in @('v2', 'v3')) {
        $appGradle = Get-Content -Raw (Join-Path $root 'src/AnPod.Receiver/app/build.gradle')
        $versionCode = if ($ReceiverCandidate -eq 'v3') { [int][regex]::Match($appGradle, '(?m)^\s*versionCode\s+(\d+)\s*$').Groups[1].Value } else { 3 }
        $versionName = if ($ReceiverCandidate -eq 'v3') { [regex]::Match($appGradle, "(?m)^\s*versionName\s+'([^']+)'\s*$").Groups[1].Value } else { '0.3.0-v2-candidate' }
        if ($ReceiverCandidate -eq 'v3') {
            foreach ($requiredPath in @($ReceiverArtifactDirectory, $NativeArtifactDirectory, $LegacyPayloadDirectory, $RustTargetDirectory)) {
                if (-not $requiredPath) { throw 'V3 packaging requires explicit isolated Receiver, Native, Legacy, and Rust target directories.' }
            }
        }
        if ($ReceiverCandidate -eq 'v2') {
        foreach ($requiredPath in @($ReceiverArtifactDirectory, $NativeArtifactDirectory, $LegacyPayloadDirectory, $RustTargetDirectory)) {
            if (-not $requiredPath) { throw 'V2 packaging requires explicit isolated Receiver, Native, Legacy, and Rust target directories.' }
        }
        }
        $artifactsPrefix = $artifactsRoot + [IO.Path]::DirectorySeparatorChar
        foreach ($sourcePath in @($receiverSource, $nativeSource, $legacyPayloadSource, $targetDir)) {
            if (-not $sourcePath.StartsWith($artifactsPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "V$ReceiverCandidate artifacts and Rust target must use isolated paths under artifacts/: $([IO.Path]::GetFileName($sourcePath))"
            }
        }
        if ($ReceiverCandidate -eq 'v2' -and (Test-Path -LiteralPath $targetDir) -and -not $ReuseExistingBuild) { throw "V2 Rust target already exists; pass -ReuseExistingBuild only when the audited build inputs are unchanged: $targetDir" }
        if ($ReceiverCandidate -eq 'v2' -and -not (Test-Path -LiteralPath $targetDir) -and $ReuseExistingBuild) { throw "Cannot reuse a missing V2 Rust target directory: $targetDir" }
        if ($ReceiverCandidate -eq 'v3' -and (Test-Path -LiteralPath $targetDir)) { throw "V3 Rust target already exists; choose a new isolated directory: $targetDir" }

        $receiverMetadata = Get-Content -Raw (Join-Path $receiverSource 'receiver.json') | ConvertFrom-Json
        if ($receiverMetadata.applicationId -ne 'app.anpod.receiver' -or $receiverMetadata.versionName -ne $versionName -or $receiverMetadata.versionCode -ne $versionCode -or $receiverMetadata.targetSdk -ne 35 -or $receiverMetadata.variant -ne 'debug' -or $receiverMetadata.sha256 -notmatch '^[0-9a-fA-F]{64}$' -or $receiverMetadata.signerSha256 -notmatch '^[0-9a-fA-F]{64}$' -or $receiverMetadata.sourceCommit -notmatch '^[0-9a-f]{40}$') {
            throw "V$ReceiverCandidate package requires its isolated Receiver APK metadata: app.anpod.receiver, code $versionCode, targetSdk 35, debug candidate signature, and source commit."
        }
        $actualReceiverHash = (Get-FileHash -LiteralPath (Join-Path $receiverSource 'receiver.apk') -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($receiverMetadata.sha256.ToLowerInvariant() -ne $actualReceiverHash) { throw "V$ReceiverCandidate Receiver APK SHA-256 does not match receiver.json." }
        $currentSourceCommit = (& git -C $root rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the current source commit.' }
        & git -C $root merge-base --is-ancestor $receiverMetadata.sourceCommit $currentSourceCommit
        if ($LASTEXITCODE -ne 0) { throw "V$ReceiverCandidate Receiver source commit is not an ancestor of the current source." }
        $receiverInputPaths = @('src/AnPod.Receiver', 'src/AnPod.AndroidProtocol', 'packaging/build-receiver.ps1', 'packaging/source-snapshot.ps1')
        $receiverCommittedChanges = @(& git -C $root diff --name-only "$($receiverMetadata.sourceCommit)..$currentSourceCommit" -- @receiverInputPaths)
        if ($LASTEXITCODE -ne 0) { throw 'Could not compare the committed Receiver build inputs.' }
        $receiverWorkingChanges = @(& git -C $root status --porcelain=v1 -- @receiverInputPaths)
        if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the current Receiver build input state.' }
        if ($ReviewWorkingTree) {
            if ($receiverMetadata.sourceInputsSha256 -notmatch '^[0-9a-f]{64}$' -or
                $receiverMetadata.sourceInputsSha256 -ne (Get-AnPodSourceSnapshot $root $receiverInputPaths)) {
                throw 'Receiver source snapshot differs from the built artifact; rebuild it before packaging.'
            }
        } elseif ($receiverCommittedChanges.Count -or $receiverWorkingChanges.Count) {
            throw "V$ReceiverCandidate Receiver build inputs changed after source commit $($receiverMetadata.sourceCommit); rebuild the Receiver artifact before packaging."
        }
        if ($ReuseExistingBuild) {
            $tauriInputPaths = @('src-tauri', 'frontend')
            $tauriCommittedChanges = @(& git -C $root diff --name-only "$($receiverMetadata.sourceCommit)..$currentSourceCommit" -- @tauriInputPaths)
            if ($LASTEXITCODE -ne 0) { throw 'Could not compare the committed Tauri/frontend build inputs.' }
            $tauriWorkingChanges = @(& git -C $root status --porcelain=v1 -- @tauriInputPaths)
            if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the current Tauri/frontend build input state.' }
            if ($tauriCommittedChanges.Count -or $tauriWorkingChanges.Count) {
                throw "Tauri/frontend build inputs changed after source commit $($receiverMetadata.sourceCommit); cannot reuse the existing Tauri build."
            }
        }

        if ($ReceiverCandidate -eq 'v3') {
            $auditedComponentCommit = '608c98c75ec04e11555e9e547caa516c96d4e0dd'
            $expectedNativeSha256 = 'eeda608983ef520f281d72c4f0021634b4e87c251ab83dea7f8dcb413068761e'
            $expectedLegacySha256 = '9552af169e4c2b2ab82c39719d3afd7c2c5dd729551d8d487e836013eeb9cfa3'
            $receiverMatchesCandidate = $receiverMetadata.versionName -eq $versionName `
                -and $receiverMetadata.versionCode -eq $versionCode `
                -and $receiverMetadata.signerSha256.ToLowerInvariant() -eq 'f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439'
            if (-not $receiverMatchesCandidate) {
                throw 'V3 review packaging requires the current Receiver version signed by the audited candidate certificate.'
            }
            foreach ($sourcePath in @($nativeSource, $legacyPayloadSource)) {
                if (-not $sourcePath.StartsWith($artifactsPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'V3 reused components must come from isolated artifacts/ directories.' }
            }
            $nativeHash = (Get-FileHash -LiteralPath (Join-Path $nativeSource 'audio_capture.dll') -Algorithm SHA256).Hash.ToLowerInvariant()
            $legacyHash = (Get-FileHash -LiteralPath (Join-Path $legacyPayloadSource 'server') -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($nativeHash -ne $expectedNativeSha256 -or $legacyHash -ne $expectedLegacySha256) {
                throw 'V3 reused Native or Legacy payload does not match the audited 608c98c candidate hashes.'
            }
            & git -C $root merge-base --is-ancestor $auditedComponentCommit $currentSourceCommit
            if ($LASTEXITCODE -ne 0) { throw 'Audited Native/Legacy source commit is not an ancestor of the current source.' }
            $componentInputPaths = @('src/AnPod.Native', 'src/AnPod.AndroidServer', 'src/AnPod.AndroidProtocol')
            $componentCommittedChanges = @(& git -C $root diff --name-only "$auditedComponentCommit..$currentSourceCommit" -- @componentInputPaths)
            if ($LASTEXITCODE -ne 0) { throw 'Could not compare the audited Native/Legacy source inputs.' }
            $componentWorkingChanges = @(& git -C $root status --porcelain=v1 -- @componentInputPaths)
            if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the audited Native/Legacy source inputs.' }
            if ($componentCommittedChanges.Count -or $componentWorkingChanges.Count) { throw 'Native/Legacy source inputs changed after the audited 608c98c build.' }
            $desktopWorkingChanges = @(& git -C $root status --porcelain=v1 -- src-tauri frontend)
            if ($LASTEXITCODE -ne 0 -or ($desktopWorkingChanges.Count -and -not $ReviewWorkingTree)) { throw 'V3 desktop build inputs must be committed and clean, or explicitly use a source-verified working-tree review build.' }
        }

        if (-not $AndroidSdkRoot) { $AndroidSdkRoot = if ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { $env:ANDROID_HOME } }
        $androidBuildTools = if ($AndroidSdkRoot) {
            Get-ChildItem -LiteralPath (Join-Path $AndroidSdkRoot 'build-tools') -Directory |
                Where-Object { (Test-Path -LiteralPath (Join-Path $_.FullName 'aapt.exe')) -and (Test-Path -LiteralPath (Join-Path $_.FullName 'apksigner.bat')) } |
                Sort-Object { [version]((Get-Content -LiteralPath (Join-Path $_.FullName 'source.properties') | Where-Object { $_ -like 'Pkg.Revision=*' } | Select-Object -First 1) -replace '^Pkg.Revision=', '') } -Descending |
                Select-Object -First 1
        }
        if (-not $androidBuildTools) { throw 'V2 packaging requires Android SDK Build-Tools with aapt.exe and apksigner.bat.' }
        $apkPath = Join-Path $receiverSource 'receiver.apk'
        $badging = & (Join-Path $androidBuildTools.FullName 'aapt.exe') dump badging $apkPath
        if ($LASTEXITCODE -ne 0) { throw 'aapt could not inspect the V2 Receiver APK.' }
        $packageLine = $badging | Where-Object { $_ -match '^package:' } | Select-Object -First 1
        if ($packageLine -notmatch "name='app\.anpod\.receiver' versionCode='$versionCode' versionName='$([regex]::Escape($versionName))'" -or ($badging -join "`n") -notmatch "targetSdkVersion:'35'") {
            throw "V$ReceiverCandidate Receiver APK identity, version, or target SDK does not match receiver.json."
        }
        $signatureOutput = & (Join-Path $androidBuildTools.FullName 'apksigner.bat') verify --print-certs $apkPath 2>&1
        $signerMatch = [regex]::Match(($signatureOutput -join "`n"), 'Signer #1 certificate SHA-256 digest: (?<sha>[0-9a-fA-F]{64})')
        if ($LASTEXITCODE -ne 0 -or -not $signerMatch.Success -or $signerMatch.Groups['sha'].Value.ToLowerInvariant() -ne $receiverMetadata.signerSha256.ToLowerInvariant()) {
            throw "V$ReceiverCandidate Receiver APK signer does not match receiver.json."
        }
    }
}
if ($IncludeReceiverRelease) {
    $receiverMetadata = Get-Content -Raw (Join-Path $receiverSource 'receiver.json') | ConvertFrom-Json
    $appGradle = Get-Content -Raw (Join-Path $root 'src/AnPod.Receiver/app/build.gradle')
    $releaseVersionCode = [int][regex]::Match($appGradle, '(?m)^\s*versionCode\s+(\d+)\s*$').Groups[1].Value
    $releaseVersionName = [regex]::Match($appGradle, "(?m)^\s*versionName\s+'([^']+)'\s*$").Groups[1].Value
    if ($receiverMetadata.applicationId -ne 'app.anpod.receiver' -or $receiverMetadata.variant -ne 'release' -or
        $receiverMetadata.versionCode -ne $releaseVersionCode -or $receiverMetadata.versionName -ne $releaseVersionName -or
        $receiverMetadata.targetSdk -ne 35 -or $receiverMetadata.sha256 -notmatch '^[0-9a-fA-F]{64}$' -or
        $receiverMetadata.signerSha256 -notmatch '^[0-9a-fA-F]{64}$' -or $receiverMetadata.sourceCommit -notmatch '^[0-9a-f]{40}$') {
        throw 'Release packaging needs a Receiver built with -Variant release for the current gradle version, targetSdk 35, and a signed artifact.'
    }
    $candidateDebugSigner = 'f5d31b5c142b8be47b345971a9993bb6a124379833e01e59feb519bda9469439'
    if ($receiverMetadata.signerSha256.ToLowerInvariant() -eq $candidateDebugSigner) {
        throw 'The audited candidate debug certificate cannot sign a release Receiver.'
    }
    $actualReceiverHash = (Get-FileHash -LiteralPath (Join-Path $receiverSource 'receiver.apk') -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($receiverMetadata.sha256.ToLowerInvariant() -ne $actualReceiverHash) { throw 'Release Receiver APK SHA-256 does not match receiver.json.' }
    $currentSourceCommit = (& git -C $root rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the current source commit.' }
    & git -C $root merge-base --is-ancestor $receiverMetadata.sourceCommit $currentSourceCommit 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Release Receiver source commit is not an ancestor of the current source.' }
    $receiverInputPaths = @('src/AnPod.Receiver', 'src/AnPod.AndroidProtocol', 'packaging/build-receiver.ps1', 'packaging/source-snapshot.ps1')
    $releaseCommittedChanges = @(& git -C $root diff --name-only "$($receiverMetadata.sourceCommit)..$currentSourceCommit" -- @receiverInputPaths)
    if ($LASTEXITCODE -ne 0) { throw 'Could not compare the committed Receiver build inputs.' }
    $releaseWorkingChanges = @(& git -C $root status --porcelain=v1 -- @receiverInputPaths)
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the current Receiver build input state.' }
    if ($releaseCommittedChanges.Count -or $releaseWorkingChanges.Count) {
        throw 'Receiver build inputs changed after the release artifact was built; rebuild it before packaging.'
    }
    if (-not $AndroidSdkRoot) { $AndroidSdkRoot = if ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { $env:ANDROID_HOME } }
    $releaseBuildTools = if ($AndroidSdkRoot) {
        Get-ChildItem -LiteralPath (Join-Path $AndroidSdkRoot 'build-tools') -Directory |
            Where-Object { (Test-Path -LiteralPath (Join-Path $_.FullName 'aapt.exe')) -and (Test-Path -LiteralPath (Join-Path $_.FullName 'apksigner.bat')) } |
            Sort-Object { [version]((Get-Content -LiteralPath (Join-Path $_.FullName 'source.properties') | Where-Object { $_ -like 'Pkg.Revision=*' } | Select-Object -First 1) -replace '^Pkg.Revision=', '') } -Descending |
            Select-Object -First 1
    }
    if (-not $releaseBuildTools) { throw 'Release packaging requires Android SDK Build-Tools with aapt.exe and apksigner.bat.' }
    $releaseApk = Join-Path $receiverSource 'receiver.apk'
    $releaseBadging = & (Join-Path $releaseBuildTools.FullName 'aapt.exe') dump badging $releaseApk
    $releaseBadgingText = $releaseBadging -join "`n"
    if ($LASTEXITCODE -ne 0 -or
        $releaseBadgingText -notmatch "name='app\.anpod\.receiver' versionCode='$releaseVersionCode' versionName='$([regex]::Escape($releaseVersionName))'" -or
        $releaseBadgingText -notmatch "targetSdkVersion:'35'") {
        throw 'aapt could not confirm the release APK identity, version, or targetSdk 35.'
    }
    $releaseSignature = & (Join-Path $releaseBuildTools.FullName 'apksigner.bat') verify --print-certs $releaseApk 2>&1
    $releaseSignerMatch = [regex]::Match(($releaseSignature -join "`n"), 'Signer #1 certificate SHA-256 digest: (?<sha>[0-9a-fA-F]{64})')
    if ($LASTEXITCODE -ne 0 -or -not $releaseSignerMatch.Success -or $releaseSignerMatch.Groups['sha'].Value.ToLowerInvariant() -ne $receiverMetadata.signerSha256.ToLowerInvariant()) {
        throw 'Release Receiver APK signer does not match receiver.json.'
    }
}

$missing = @($required | Where-Object { $source = if ($_ -eq 'audio_capture.dll') { $nativeSource } elseif ($_ -eq 'server') { $legacyPayloadSource } else { $deps }; -not (Test-Path -LiteralPath (Join-Path $source $_) -PathType Leaf) })
if ($missing.Count) { throw "Missing runtime files in deps/: $($missing -join ', ')" }

if (-not $ReuseExistingBuild) {
    Push-Location (Join-Path $root 'frontend')
    try {
        if ($npm) {
            Write-Host "Building frontend with $($npm.Name)..." -ForegroundColor Cyan
            & $npm.Source run build
            if ($LASTEXITCODE -ne 0) { throw "Frontend build failed with exit code $LASTEXITCODE." }
        } else {
            Write-Host 'Building frontend with the already-installed local TypeScript and Vite commands...' -ForegroundColor Cyan
            & $tsc -b
            if ($LASTEXITCODE -ne 0) { throw "TypeScript build failed with exit code $LASTEXITCODE." }
            & $vite build
            if ($LASTEXITCODE -ne 0) { throw "Vite build failed with exit code $LASTEXITCODE." }
        }
    }
    finally { Pop-Location }
}

if (-not $ReuseExistingBuild) {
    $oldCargoTargetDir = $env:CARGO_TARGET_DIR
    $oldRustFlags = $env:RUSTFLAGS
    $oldEncodedRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
    try {
        $remap = "--remap-path-prefix=$root=."
        if (-not [string]::IsNullOrWhiteSpace($oldEncodedRustFlags)) {
            $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedRustFlags + [char]31 + $remap
        } else {
            $rustFlagArgs = @()
            if (-not [string]::IsNullOrWhiteSpace($oldRustFlags)) {
                $rustFlagArgs = @($oldRustFlags -split '\s+' | Where-Object { $_.Length -gt 0 })
            }
            $rustFlagArgs += $remap
            $env:CARGO_ENCODED_RUSTFLAGS = [string]::Join([string][char]31, [string[]]$rustFlagArgs)
        }
        $env:RUSTFLAGS = $oldRustFlags
        $env:CARGO_TARGET_DIR = $targetDir
        $cargoArguments = @('build', '--release', '--locked', '--manifest-path', (Join-Path $root 'src-tauri\Cargo.toml'))
        if ($Offline) { $cargoArguments += '--offline' }
        $previousErrorAction = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        & $cargo.Source @cargoArguments 2>&1 | ForEach-Object { Write-Host $_ }
        $ErrorActionPreference = $previousErrorAction
        if ($LASTEXITCODE -ne 0) { throw "Tauri release build failed with exit code $LASTEXITCODE." }
    }
    finally {
        $env:RUSTFLAGS = $oldRustFlags
        $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedRustFlags
        $env:CARGO_TARGET_DIR = $oldCargoTargetDir
    }
}

$exe = Join-Path $targetDir 'release\anpod-tauri.exe'
if ((Get-AnPodSourceSnapshot $root $desktopInputPaths) -ne $desktopInputsSha256) { throw 'Desktop source inputs changed during the build.' }
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Release executable was not created: $exe" }

if (Test-Path -LiteralPath $packageDir) {
    $existing = Get-Item -LiteralPath $packageDir -Force
    if ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing to recursively remove a redirected package path: $packageDir" }
    $resolvedPackage = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $packageDir).Path)
    if (-not $resolvedPackage.StartsWith($artifactsRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove a package outside artifacts/: $resolvedPackage"
    }
    Remove-Item -LiteralPath $resolvedPackage -Recurse -Force
}
if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
New-Item -ItemType Directory -Path $packageDir -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $packageDir 'AnPod.exe')
foreach ($name in $required[0..4]) { $source = if ($name -eq 'audio_capture.dll') { $nativeSource } elseif ($name -eq 'server') { $legacyPayloadSource } else { $deps }; Copy-Item -LiteralPath (Join-Path $source $name) -Destination (Join-Path $packageDir $name) }
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $packageDir
Copy-Item -LiteralPath (Join-Path $root 'THIRD_PARTY_NOTICES.md') -Destination $packageDir
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination $packageDir
Copy-Item -LiteralPath (Join-Path $root 'frontend\public\icon.png') -Destination (Join-Path $packageDir 'icon.png')
Copy-Item -LiteralPath (Join-Path $root 'LICENSES') -Destination $packageDir -Recurse
Copy-Item -LiteralPath (Join-Path $deps 'PLATFORM_TOOLS_NOTICE.txt') -Destination (Join-Path $packageDir 'LICENSES\PLATFORM_TOOLS_NOTICE.txt')
Copy-Item -LiteralPath (Join-Path $root 'upstream') -Destination $packageDir -Recurse
if ($IncludeReceiverCandidate -or $IncludeReceiverRelease) {
    foreach ($name in $receiverFiles) { Copy-Item -LiteralPath (Join-Path $receiverSource $name) -Destination (Join-Path $packageDir $name) }
}
if ($ReceiverCandidate -eq 'v3' -and $IncludeReceiverCandidate) {
    $buildMetadata = [ordered]@{
        schemaVersion = 1
        candidate = 'receiver-v3-usb-oneclick-review'
        productVersion = $Version
        buildCommit = $currentSourceCommit
        workingTreeReview = [bool]$ReviewWorkingTree
        desktop = [ordered]@{ sha256 = (Get-FileHash -LiteralPath (Join-Path $packageDir 'AnPod.exe') -Algorithm SHA256).Hash.ToLowerInvariant(); sourceCommit = $currentSourceCommit; sourceInputsSha256 = $desktopInputsSha256 }
        receiver = [ordered]@{ sha256 = $receiverMetadata.sha256.ToLowerInvariant(); sourceCommit = $receiverMetadata.sourceCommit; sourceInputsSha256 = $receiverMetadata.sourceInputsSha256; sourceModified = $receiverMetadata.sourceModified; versionCode = $receiverMetadata.versionCode; versionName = $receiverMetadata.versionName; targetSdk = $receiverMetadata.targetSdk; variant = $receiverMetadata.variant; signerSha256 = $receiverMetadata.signerSha256.ToLowerInvariant() }
        native = [ordered]@{ sha256 = $expectedNativeSha256; sourceCommit = $auditedComponentCommit }
        legacyServer = [ordered]@{ sha256 = $expectedLegacySha256; sourceCommit = $auditedComponentCommit }
    }
    $buildMetadataPath = Join-Path $packageDir 'build-metadata.json'
    [IO.File]::WriteAllText($buildMetadataPath, ($buildMetadata | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
}
if ($IncludeReceiverRelease) {
    $buildMetadata = [ordered]@{
        schemaVersion = 1
        candidate = 'receiver-release'
        productVersion = $Version
        buildCommit = $currentSourceCommit
        workingTreeReview = $false
        workingTreeDirty = [bool](@(& git -C $root status --porcelain=v1).Count)
        desktop = [ordered]@{ sha256 = (Get-FileHash -LiteralPath (Join-Path $packageDir 'AnPod.exe') -Algorithm SHA256).Hash.ToLowerInvariant(); sourceCommit = $currentSourceCommit; sourceInputsSha256 = $desktopInputsSha256 }
        receiver = [ordered]@{ sha256 = $receiverMetadata.sha256.ToLowerInvariant(); sourceCommit = $receiverMetadata.sourceCommit; sourceInputsSha256 = $receiverMetadata.sourceInputsSha256; sourceModified = $receiverMetadata.sourceModified; versionCode = $receiverMetadata.versionCode; versionName = $receiverMetadata.versionName; targetSdk = $receiverMetadata.targetSdk; variant = $receiverMetadata.variant; signerSha256 = $receiverMetadata.signerSha256.ToLowerInvariant() }
    }
    $buildMetadataPath = Join-Path $packageDir 'build-metadata.json'
    [IO.File]::WriteAllText($buildMetadataPath, ($buildMetadata | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
}

$privatePaths = @($root, $root.Replace('\', '/'), $env:USERPROFILE, $env:ANDROID_SDK_ROOT, $env:ANDROID_HOME, $env:JAVA_HOME, $env:CARGO_HOME, $env:RUSTUP_HOME, $env:GRADLE_USER_HOME) |
    Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
    ForEach-Object { $_.ToLowerInvariant(); $_.Replace('\', '/').ToLowerInvariant() } |
    Select-Object -Unique
$prohibitedNames = @('AnPod_plan_v2.md', 'AnPod_plan_v3.md', 'plan.MD', 'plan.md', 'AnPod_handoff_2026-10-14.md', 'probe', 'keystore', '.jks', '.p12', '.pem', '.key', '.pdb', '.log', 'audio-sample', 'adbkey')
$packageFiles = @(Get-ChildItem -LiteralPath $packageDir -Recurse -File)
foreach ($file in $packageFiles) {
    $relativePath = $file.FullName.Substring($packageDir.Length + 1)
    if ($prohibitedNames | Where-Object { $relativePath -like "*$_*" }) { throw "Prohibited file in candidate package: $relativePath" }
    $bytes = [IO.File]::ReadAllBytes($file.FullName)
    $ascii = [Text.Encoding]::ASCII.GetString($bytes).ToLowerInvariant()
    $unicode = [Text.Encoding]::Unicode.GetString($bytes).ToLowerInvariant()
    if ($privatePaths | Where-Object { $ascii.Contains($_) -or $unicode.Contains($_) }) { throw "Private build path remains in candidate file: $relativePath" }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$sourceRoot = (Resolve-Path -LiteralPath $packageDir).Path.TrimEnd('\', '/')
$archive = [System.IO.Compression.ZipFile]::Open($zipPath, 'Create')
try {
    Get-ChildItem -LiteralPath $sourceRoot -Recurse -File | ForEach-Object {
        $relative = $_.FullName.Substring($sourceRoot.Length + 1) -replace '\\', '/'
        [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $_.FullName, $relative, [System.IO.Compression.CompressionLevel]::Optimal)
    }
}
finally { $archive.Dispose() }

$archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
try {
    foreach ($entry in $archive.Entries) {
        $entryPath = $entry.FullName
        if ([IO.Path]::IsPathRooted($entryPath) -or $entryPath -match '(^|/)\.\.(/|$)') { throw "Unsafe ZIP entry path: $entryPath" }
        if ($prohibitedNames | Where-Object { $entryPath -like "*$_*" }) { throw "Prohibited ZIP entry: $entryPath" }
        if (-not $entryPath) { continue }
        $entryStream = $entry.Open()
        $memory = [IO.MemoryStream]::new()
        try {
            $entryStream.CopyTo($memory)
            $bytes = $memory.ToArray()
        }
        finally { $entryStream.Dispose(); $memory.Dispose() }
        $ascii = [Text.Encoding]::ASCII.GetString($bytes).ToLowerInvariant()
        $unicode = [Text.Encoding]::Unicode.GetString($bytes).ToLowerInvariant()
        if ($privatePaths | Where-Object { $ascii.Contains($_) -or $unicode.Contains($_) }) { throw "Private build path remains in ZIP entry: $entryPath" }
    }
}
finally { $archive.Dispose() }

Write-Host "Package directory: $packageDir" -ForegroundColor Green
Write-Host "Portable ZIP: $zipPath" -ForegroundColor Green
$sha256 = [System.Security.Cryptography.SHA256]::Create()
$stream = [System.IO.File]::OpenRead($zipPath)
try { $hash = ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '') }
finally { $stream.Dispose(); $sha256.Dispose() }
Write-Host "SHA256: $hash"
