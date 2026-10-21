[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory)]
    [ValidateSet('patch', 'minor')]
    [string]$Change
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$utf8 = [Text.UTF8Encoding]::new($false)
$cargoPath = Join-Path $root 'src-tauri/Cargo.toml'
$lockPath = Join-Path $root 'src-tauri/Cargo.lock'
$configPath = Join-Path $root 'src-tauri/tauri.conf.json'
$receiverPath = Join-Path $root 'src/AnPod.Receiver/app/build.gradle'
$cargo = [IO.File]::ReadAllText($cargoPath)
$lock = [IO.File]::ReadAllText($lockPath)
$config = [IO.File]::ReadAllText($configPath)
$receiver = [IO.File]::ReadAllText($receiverPath)
$cargoVersion = [regex]::Match($cargo, '(?m)^version = "(?<value>\d+\.\d+\.\d+)"\r?$')
$lockVersion = [regex]::Match($lock, '(?m)^name = "anpod-tauri"\r?\nversion = "(?<value>\d+\.\d+\.\d+)"')
$receiverVersion = [regex]::Match($receiver, "(?m)^\s*versionName '(?<value>\d+\.\d+\.\d+)(?<suffix>[^']*)'\s*$")
$receiverCode = [regex]::Match($receiver, '(?m)^\s*versionCode (?<value>\d+)\s*$')
if (-not $cargoVersion.Success -or -not $lockVersion.Success -or
        -not $receiverVersion.Success -or -not $receiverCode.Success) {
    throw 'Could not read the current desktop/Receiver versions; no files changed.'
}
$oldDesktop = $cargoVersion.Groups['value'].Value
if (($config | ConvertFrom-Json).version -ne $oldDesktop -or $lockVersion.Groups['value'].Value -ne $oldDesktop) {
    throw 'Cargo.toml, Cargo.lock and Tauri versions disagree; no files changed.'
}
$oldReceiver = $receiverVersion.Groups['value'].Value
$desktopParts = $oldDesktop.Split('.')
$receiverParts = $oldReceiver.Split('.')
if ($Change -eq 'minor') {
    $newDesktop = '{0}.{1}.0' -f $desktopParts[0], (1 + [int]$desktopParts[1])
    $newReceiver = '{0}.{1}.0' -f $receiverParts[0], (1 + [int]$receiverParts[1])
} else {
    $newDesktop = '{0}.{1}.{2}' -f $desktopParts[0], $desktopParts[1], (1 + [int]$desktopParts[2])
    $newReceiver = '{0}.{1}.{2}' -f $receiverParts[0], $receiverParts[1], (1 + [int]$receiverParts[2])
}
$newCode = 1 + [int]$receiverCode.Groups['value'].Value
if ($newCode -gt 2100000000) { throw 'Receiver versionCode exceeds the Android limit; no files changed.' }
$newReceiverName = $newReceiver + $receiverVersion.Groups['suffix'].Value
$cargo = $cargo.Replace($cargoVersion.Value, $cargoVersion.Value.Replace($oldDesktop, $newDesktop))
$lock = $lock.Replace($lockVersion.Value, $lockVersion.Value.Replace($oldDesktop, $newDesktop))
$config = $config -replace ('("version"\s*:\s*")' + [regex]::Escape($oldDesktop) + '(")'), ('${1}' + $newDesktop + '${2}')
$receiver = $receiver.Replace($receiverVersion.Value,
    $receiverVersion.Value.Replace($receiverVersion.Groups['value'].Value + $receiverVersion.Groups['suffix'].Value, $newReceiverName))
$receiver = $receiver.Replace($receiverCode.Value,
    $receiverCode.Value.Replace($receiverCode.Groups['value'].Value, [string]$newCode))
if ($PSCmdlet.ShouldProcess('AnPod candidate source versions', "Desktop $oldDesktop -> $newDesktop; Receiver $oldReceiver -> $newReceiverName / code $newCode")) {
    [IO.File]::WriteAllText($cargoPath, $cargo, $utf8)
    [IO.File]::WriteAllText($lockPath, $lock, $utf8)
    [IO.File]::WriteAllText($configPath, $config, $utf8)
    [IO.File]::WriteAllText($receiverPath, $receiver, $utf8)
    Write-Output "Desktop $newDesktop; Receiver $newReceiverName / code $newCode"
}
