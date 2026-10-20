<#
Explicit experimental AOA mode switch for the product Receiver identity.
Supported source device is limited to iQOO 11 V2243A (VID_2D95 PID_6013 MI_02).
This sends AOA requests only; it does not enumerate afterward, stop ADB, or change drivers.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Write-Host 'AnPod USB Direct mode switch (experimental; supported target: iQOO 11 V2243A only).' -ForegroundColor Cyan
Write-Host 'The phone must have a matching AnPod Receiver build installed. ADB/MTP may disconnect.' -ForegroundColor Yellow
Write-Host 'This sends GET_PROTOCOL (51), the AnPod / AnPod USB Direct identity strings (52), then START_ACCESSORY (53).'
Write-Host 'No driver is installed or changed, and this script does not stop ADB or inspect other devices.'
$adbProcesses = @(Get-Process -Name adb -ErrorAction SilentlyContinue)
if ($adbProcesses.Count -gt 0) {
    Write-Host 'STOP: adb.exe is running. Close AnPod/ADB clients and stop the ADB server manually, then retry; this script will not stop it.' -ForegroundColor Yellow
    exit 2
}
$answer = Read-Host 'To send the USB mode-switch requests, type exactly SWITCH AOA'
if ($answer -cne 'SWITCH AOA') {
    Write-Host 'Cancelled; no USB requests sent.'
    exit 1
}

try {
    $helper = Join-Path $PSScriptRoot 'AoaHost.cs'
    if (-not (Test-Path -LiteralPath $helper -PathType Leaf)) {
        $helper = Join-Path $PSScriptRoot '..\AnPod_AOA_Phase2_Probe\Windows\AoaHost.cs'
    }
    Add-Type -TypeDefinition (Get-Content -LiteralPath $helper -Raw -Encoding UTF8) -Language CSharp -ErrorAction Stop
    Write-Host ([AnPodAoaPhase2]::SwitchAnPodReceiver())
} catch {
    Write-Host ('ERROR: ' + $_.Exception.Message) -ForegroundColor Red
    Write-Host 'The script did not modify USB drivers. Reconnect the cable to restore the phone to its original USB mode if needed.'
    exit 1
}
