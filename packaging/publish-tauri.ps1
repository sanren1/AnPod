[CmdletBinding()]
param(
    [string]$Version = '1.0.4',
    [switch]$IncludeReceiverCandidate,
    [ValidateSet('p1', 'p2', 'v2')]
    [string]$ReceiverCandidate = 'p1',
    [string]$ReceiverArtifactDirectory,
    [string]$NativeArtifactDirectory,
    [string]$LegacyPayloadDirectory,
    [string]$RustTargetDirectory,
    [string]$AndroidSdkRoot,
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw "Invalid package version: $Version" }
$root = Split-Path -Parent $PSScriptRoot
$deps = Join-Path $root 'deps'
$artifacts = Join-Path $root 'artifacts'
$packageName = if ($IncludeReceiverCandidate) { "AnPod-$Version-receiver-$ReceiverCandidate-candidate-tauri-win-x64-portable" } else { "AnPod-$Version-tauri-win-x64-portable" }
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
if ($IncludeReceiverCandidate -and $ReceiverCandidate -eq 'v2' -and ((Test-Path -LiteralPath $packageDir) -or (Test-Path -LiteralPath $zipPath))) {
    throw "Refusing to overwrite an existing v2 candidate path: $packageName"
}

if ($ReceiverCandidate -in @('p2', 'v2') -and $IncludeReceiverCandidate) {
    if ($ReceiverCandidate -eq 'v2' -and -not $RustTargetDirectory) { throw 'V2 packaging requires an explicit isolated RustTargetDirectory.' }
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
if (-not $npm -and (-not (Test-Path -LiteralPath $tsc -PathType Leaf) -or -not (Test-Path -LiteralPath $vite -PathType Leaf))) { throw 'npm or the already-installed local TypeScript/Vite commands are required.' }
if (-not $cargo) { throw 'cargo was not found on PATH. Install Rust stable 1.90 or newer.' }

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
    if ($ReceiverCandidate -eq 'v2') {
        foreach ($requiredPath in @($ReceiverArtifactDirectory, $NativeArtifactDirectory, $LegacyPayloadDirectory, $RustTargetDirectory)) {
            if (-not $requiredPath) { throw 'V2 packaging requires explicit isolated Receiver, Native, Legacy, and Rust target directories.' }
        }
        $artifactsPrefix = $artifactsRoot + [IO.Path]::DirectorySeparatorChar
        foreach ($sourcePath in @($receiverSource, $nativeSource, $legacyPayloadSource, $targetDir)) {
            if (-not $sourcePath.StartsWith($artifactsPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "V2 artifacts and Rust target must use isolated paths under artifacts/: $([IO.Path]::GetFileName($sourcePath))"
            }
        }
        if (Test-Path -LiteralPath $targetDir) { throw "Refusing to reuse a V2 Rust target directory: $targetDir" }

        $receiverMetadata = Get-Content -Raw (Join-Path $receiverSource 'receiver.json') | ConvertFrom-Json
        if ($receiverMetadata.applicationId -ne 'app.anpod.receiver' -or $receiverMetadata.versionName -ne '0.3.0-v2-candidate' -or $receiverMetadata.versionCode -ne 3 -or $receiverMetadata.targetSdk -ne 35 -or $receiverMetadata.variant -ne 'debug' -or $receiverMetadata.sha256 -notmatch '^[0-9a-fA-F]{64}$' -or $receiverMetadata.signerSha256 -notmatch '^[0-9a-fA-F]{64}$' -or $receiverMetadata.sourceCommit -notmatch '^[0-9a-f]{40}$') {
            throw 'V2 package requires the isolated v2 Receiver APK metadata: app.anpod.receiver, code 3, targetSdk 35, debug candidate signature, and source commit.'
        }
        $actualReceiverHash = (Get-FileHash -LiteralPath (Join-Path $receiverSource 'receiver.apk') -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($receiverMetadata.sha256.ToLowerInvariant() -ne $actualReceiverHash) { throw 'V2 Receiver APK SHA-256 does not match receiver.json.' }
        $currentSourceCommit = (& git -C $root rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0 -or $receiverMetadata.sourceCommit -ne $currentSourceCommit) {
            throw 'V2 Receiver source commit does not match the current committed source.'
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
        if ($packageLine -notmatch "name='app\.anpod\.receiver' versionCode='3' versionName='0\.3\.0-v2-candidate'" -or ($badging -join "`n") -notmatch "targetSdkVersion:'35'") {
            throw 'V2 Receiver APK identity, version, or target SDK does not match receiver.json.'
        }
        $signatureOutput = & (Join-Path $androidBuildTools.FullName 'apksigner.bat') verify --print-certs $apkPath 2>&1
        $signerMatch = [regex]::Match(($signatureOutput -join "`n"), 'Signer #1 certificate SHA-256 digest: (?<sha>[0-9a-fA-F]{64})')
        if ($LASTEXITCODE -ne 0 -or -not $signerMatch.Success -or $signerMatch.Groups['sha'].Value.ToLowerInvariant() -ne $receiverMetadata.signerSha256.ToLowerInvariant()) {
            throw 'V2 Receiver APK signer does not match receiver.json.'
        }
    }
}
$missing = @($required | Where-Object { $source = if ($_ -eq 'audio_capture.dll') { $nativeSource } elseif ($_ -eq 'server') { $legacyPayloadSource } else { $deps }; -not (Test-Path -LiteralPath (Join-Path $source $_) -PathType Leaf) })
if ($missing.Count) { throw "Missing runtime files in deps/: $($missing -join ', ')" }

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
    & $cargo.Source @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw "Tauri release build failed with exit code $LASTEXITCODE." }
}
finally {
    $env:RUSTFLAGS = $oldRustFlags
    $env:CARGO_ENCODED_RUSTFLAGS = $oldEncodedRustFlags
    $env:CARGO_TARGET_DIR = $oldCargoTargetDir
}

$exe = Join-Path $targetDir 'release\anpod-tauri.exe'
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
Copy-Item -LiteralPath (Join-Path $root 'frontend\public\icon.png') -Destination (Join-Path $packageDir 'icon.png')
Copy-Item -LiteralPath (Join-Path $root 'LICENSES') -Destination $packageDir -Recurse
Copy-Item -LiteralPath (Join-Path $deps 'PLATFORM_TOOLS_NOTICE.txt') -Destination (Join-Path $packageDir 'LICENSES\PLATFORM_TOOLS_NOTICE.txt')
Copy-Item -LiteralPath (Join-Path $root 'upstream') -Destination $packageDir -Recurse
if ($IncludeReceiverCandidate) {
    foreach ($name in $receiverFiles) { Copy-Item -LiteralPath (Join-Path $receiverSource $name) -Destination (Join-Path $packageDir $name) }
}

$privatePaths = @($root, $root.Replace('\', '/'), $env:USERPROFILE, $env:ANDROID_SDK_ROOT, $env:ANDROID_HOME, $env:JAVA_HOME, $env:CARGO_HOME, $env:RUSTUP_HOME, $env:GRADLE_USER_HOME) |
    Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
    ForEach-Object { $_.ToLowerInvariant(); $_.Replace('\', '/').ToLowerInvariant() } |
    Select-Object -Unique
$prohibitedNames = @('AnPod_plan_v2.md', 'probe', 'keystore', '.jks', '.p12', '.pem', '.key', '.pdb', '.log', 'audio-sample', 'adbkey')
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
