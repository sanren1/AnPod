$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$temporary = [IO.Path]::GetFullPath((Join-Path $root '.tmp-version-check'))
if (Test-Path -LiteralPath $temporary) { throw 'Version check directory already exists.' }
$paths = @('packaging/update-version.ps1', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock',
    'src-tauri/tauri.conf.json', 'src/AnPod.Receiver/app/build.gradle')
try {
    foreach ($path in $paths) {
        $destination = Join-Path $temporary $path
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $root $path) -Destination $destination
    }
    $script = Join-Path $temporary 'packaging/update-version.ps1'
    $config = Join-Path $temporary 'src-tauri/tauri.conf.json'
    $receiver = Join-Path $temporary 'src/AnPod.Receiver/app/build.gradle'
    $before = (Get-Content -Raw $config | ConvertFrom-Json).version
    $parts = $before.Split('.')
    $receiverBefore = Get-Content -Raw $receiver
    $receiverParts = [regex]::Match($receiverBefore, "versionName '(\d+)\.(\d+)\.(\d+)([^']*)'")
    $codeBefore = [int][regex]::Match($receiverBefore, 'versionCode (\d+)').Groups[1].Value
    & $script -Change patch -WhatIf
    if ((Get-Content -Raw $config | ConvertFrom-Json).version -ne $before) { throw 'WhatIf changed the version.' }
    & $script -Change patch
    $expectedPatch = '{0}.{1}.{2}' -f $parts[0], $parts[1], (1 + [int]$parts[2])
    if ((Get-Content -Raw $config | ConvertFrom-Json).version -ne $expectedPatch) { throw 'Patch increment failed.' }
    & $script -Change minor
    $expectedMinor = '{0}.{1}.0' -f $parts[0], (1 + [int]$parts[1])
    $expectedReceiver = '{0}.{1}.0{2}' -f $receiverParts.Groups[1].Value,
        (1 + [int]$receiverParts.Groups[2].Value), $receiverParts.Groups[4].Value
    if ((Get-Content -Raw $config | ConvertFrom-Json).version -ne $expectedMinor -or
        (Get-Content -Raw $receiver) -notmatch [regex]::Escape("versionName '$expectedReceiver'") -or
        (Get-Content -Raw $receiver) -notmatch "versionCode $($codeBefore + 2)\s") {
        throw 'Minor increment, patch reset, suffix or Receiver code failed.'
    }
    if ((Get-Content -Raw (Join-Path $temporary 'src-tauri/Cargo.toml')) -notmatch [regex]::Escape("version = `"$expectedMinor`"") -or
        (Get-Content -Raw (Join-Path $temporary 'src-tauri/Cargo.lock')) -notmatch ('name = "anpod-tauri"\r?\nversion = "' + [regex]::Escape($expectedMinor) + '"')) {
        throw 'Desktop manifests disagree after version update.'
    }
    [IO.File]::WriteAllText($config, '{"version":"99.0.0"}')
    $hashes = @($paths | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $temporary $_)).Hash })
    $rejected = $false
    try { & $script -Change patch } catch { $rejected = $true }
    if (-not $rejected -or (Compare-Object $hashes @($paths | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $temporary $_)).Hash }))) {
        throw 'Inconsistent version inputs were not rejected without mutation.'
    }
    Write-Output 'VERSION_CHECK=PASS'
} finally {
    if ($temporary -ne [IO.Path]::GetFullPath((Join-Path $root '.tmp-version-check'))) { throw 'Unexpected cleanup path.' }
    if (Test-Path -LiteralPath $temporary) {
        if ((Get-Item -LiteralPath $temporary -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing redirected cleanup path.' }
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}
