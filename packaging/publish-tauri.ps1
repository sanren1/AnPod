[CmdletBinding()]
param(
    [string]$Version = '1.0.4',
    [switch]$IncludeReceiverCandidate,
    [ValidateSet('p1', 'p2')]
    [string]$ReceiverCandidate = 'p1',
    [string]$ReceiverArtifactDirectory,
    [string]$NativeArtifactDirectory,
    [string]$LegacyPayloadDirectory,
    [string]$RustTargetDirectory
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

if ($ReceiverCandidate -eq 'p2' -and $IncludeReceiverCandidate) {
    if (-not $RustTargetDirectory) { $RustTargetDirectory = 'artifacts\p2-tauri-target' }
    $targetDir = if ([IO.Path]::IsPathRooted($RustTargetDirectory)) { [IO.Path]::GetFullPath($RustTargetDirectory) } else { [IO.Path]::GetFullPath((Join-Path $root $RustTargetDirectory)) }
} else {
    if (-not $RustTargetDirectory) { $RustTargetDirectory = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'src-tauri\target' } }
    $targetDir = [System.IO.Path]::GetFullPath($RustTargetDirectory)
}

$npm = Get-Command npm -ErrorAction SilentlyContinue
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $npm) { throw 'npm was not found on PATH. Install Node.js.' }
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
    if ($missingReceiver.Count) { throw "Receiver candidate needs both files in deps/: $($missingReceiver -join ', ')" }
    if ($ReceiverCandidate -eq 'p2') {
        $receiverMetadata = Get-Content -Raw (Join-Path $receiverSource 'receiver.json') | ConvertFrom-Json
        if ($receiverMetadata.versionName -ne '0.2.0-p2-candidate' -or $receiverMetadata.versionCode -ne 2) { throw 'P2 package requires a version 0.2.0-p2-candidate / code 2 Receiver artifact.' }
    }
}
$missing = @($required | Where-Object { $source = if ($_ -eq 'audio_capture.dll') { $nativeSource } elseif ($_ -eq 'server') { $legacyPayloadSource } else { $deps }; -not (Test-Path -LiteralPath (Join-Path $source $_) -PathType Leaf) })
if ($missing.Count) { throw "Missing runtime files in deps/: $($missing -join ', ')" }

Push-Location (Join-Path $root 'frontend')
try {
    & $npm.Source run build
    if ($LASTEXITCODE -ne 0) { throw "Frontend build failed with exit code $LASTEXITCODE." }
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
    & $cargo.Source build --release --manifest-path (Join-Path $root 'src-tauri\Cargo.toml')
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

$privatePaths = @($root.ToLowerInvariant(), $root.Replace('\', '/').ToLowerInvariant())
$scanFiles = @($exe, (Join-Path $nativeSource 'audio_capture.dll'), (Join-Path $legacyPayloadSource 'server'))
if ($IncludeReceiverCandidate) { $scanFiles += @((Join-Path $receiverSource 'receiver.apk'), (Join-Path $packageDir 'receiver.json')) }
foreach ($file in $scanFiles) {
    $bytes = [IO.File]::ReadAllBytes($file)
    $ascii = [Text.Encoding]::ASCII.GetString($bytes).ToLowerInvariant()
    $unicode = [Text.Encoding]::Unicode.GetString($bytes).ToLowerInvariant()
    if ($privatePaths | Where-Object { $ascii.Contains($_) -or $unicode.Contains($_) }) { throw "Private build path remains embedded in candidate binary: $([IO.Path]::GetFileName($file))" }
}
Get-ChildItem -LiteralPath $packageDir -Recurse -File | Where-Object { $_.Extension -in @('.txt', '.md', '.json', '.xml', '.html', '.js', '.css') } | ForEach-Object {
    $text = [IO.File]::ReadAllText($_.FullName).ToLowerInvariant()
    if ($privatePaths | Where-Object { $text.Contains($_) }) { throw "Private build path remains in candidate text: $($_.Name)" }
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

Write-Host "Package directory: $packageDir" -ForegroundColor Green
Write-Host "Portable ZIP: $zipPath" -ForegroundColor Green
$sha256 = [System.Security.Cryptography.SHA256]::Create()
$stream = [System.IO.File]::OpenRead($zipPath)
try { $hash = ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '') }
finally { $stream.Dispose(); $sha256.Dispose() }
Write-Host "SHA256: $hash"
