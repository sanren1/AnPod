[CmdletBinding()]
param([string]$Version = '1.0.4')

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') { throw "Invalid package version: $Version" }
$root = Split-Path -Parent $PSScriptRoot
$deps = Join-Path $root 'deps'
$artifacts = Join-Path $root 'artifacts'
$packageName = "AnPod-$Version-tauri-win-x64-portable"
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

if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'src-tauri\target' }
$targetDir = [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)

$npm = Get-Command npm -ErrorAction SilentlyContinue
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $npm) { throw 'npm was not found on PATH. Install Node.js.' }
if (-not $cargo) { throw 'cargo was not found on PATH. Install Rust stable 1.90 or newer.' }

$required = @('adb.exe', 'AdbWinApi.dll', 'AdbWinUsbApi.dll', 'audio_capture.dll', 'server', 'PLATFORM_TOOLS_NOTICE.txt')
$missing = @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $deps $_) -PathType Leaf) })
if ($missing.Count) { throw "Missing runtime files in deps/: $($missing -join ', ')" }

Push-Location (Join-Path $root 'frontend')
try {
    & $npm.Source run build
    if ($LASTEXITCODE -ne 0) { throw "Frontend build failed with exit code $LASTEXITCODE." }
}
finally { Pop-Location }

$env:CARGO_TARGET_DIR = $targetDir
& $cargo.Source build --release --manifest-path (Join-Path $root 'src-tauri\Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw "Tauri release build failed with exit code $LASTEXITCODE." }

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
foreach ($name in $required[0..4]) { Copy-Item -LiteralPath (Join-Path $deps $name) -Destination (Join-Path $packageDir $name) }
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $packageDir
Copy-Item -LiteralPath (Join-Path $root 'THIRD_PARTY_NOTICES.md') -Destination $packageDir
Copy-Item -LiteralPath (Join-Path $root 'LICENSES') -Destination $packageDir -Recurse
Copy-Item -LiteralPath (Join-Path $deps 'PLATFORM_TOOLS_NOTICE.txt') -Destination (Join-Path $packageDir 'LICENSES\PLATFORM_TOOLS_NOTICE.txt')
Copy-Item -LiteralPath (Join-Path $root 'upstream') -Destination $packageDir -Recurse

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
