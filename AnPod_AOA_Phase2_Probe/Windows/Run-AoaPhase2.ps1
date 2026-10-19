<#
AnPod AOA second-stage experimental probe for one iQOO 11 (VID_2D95 PID_6013).
Switch mode ONLY after typed acknowledgement. Does not touch drivers or AnPod files.
Commands: -Mode Preflight, -Mode Switch, -Mode Inspect, -Mode Bulk -Rounds 64.
Use Windows PowerShell 5.1 or PowerShell 7. Path is relative to script, not CWD.
#>
[CmdletBinding()]
param(
    [ValidateSet('Preflight','Switch','Inspect','Bulk')][string]$Mode = 'Preflight',
    [ValidateRange(1,512)][int]$Rounds = 64
)
$ErrorActionPreference = 'Stop'
Write-Host 'AnPod AOA Phase-2 Probe (EXPERIMENTAL)' -ForegroundColor Cyan
Write-Host "Mode: $Mode; does not install/replace drivers."
if ($Mode -eq 'Switch') {
    $adbProcesses = @(Get-Process -Name adb -ErrorAction SilentlyContinue)
    if ($adbProcesses.Count -gt 0) {
        Write-Host 'STOP: adb.exe is still running. Exit AnPod/Android Studio and run adb kill-server; do not restart adb until the switch.' -ForegroundColor Yellow
        exit 2
    }
    Write-Host 'WARNING: will send AOA SEND_STRING(52) and START_ACCESSORY(53).' -ForegroundColor Yellow
    Write-Host 'USB will reconnect; MTP/ADB may disappear; manual unplug/replug may be needed.'
    Write-Host 'You must install and open the AndroidProbe app BEFORE running this switch.'
    $answer = Read-Host 'To proceed, type exactly SWITCH AOA'
    if ($answer -cne 'SWITCH AOA') { Write-Host 'Cancelled; no USB requests sent.'; exit 1 }
}
try {
    Add-Type -TypeDefinition (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'AoaHost.cs') -Raw -Encoding UTF8) -Language CSharp -ErrorAction Stop
    switch ($Mode) {
        Preflight { Write-Host ([AnPodAoaPhase2]::Preflight()) }
        Switch {
            Write-Host ([AnPodAoaPhase2]::SwitchAccessory())
            Start-Sleep -Seconds 4
            Write-Host '--- Detected Google AOA devices (VID_18D1) ---'
            Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object {
                $_.InstanceId -match 'VID_18D1&PID_2D0[01]'
            } | Select-Object FriendlyName,Status,Class,@{Name='UsbInterface';Expression={
                if ($_.InstanceId -match '(VID_18D1&PID_2D0[01](?:&MI_[0-9A-F]{2})?)') { $matches[1] } else { 'AOA detected' }
            }} | Format-List
            Write-Host 'If device is present, open the AndroidProbe app on the phone, approve USB permission, then run -Mode Inspect.'
        }
        Inspect {
            Write-Host ([AnPodAoaPhase2]::InspectAccessory())
            Write-Host '--- Related present PnP devices ---'
            Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object {
                $_.InstanceId -match 'VID_18D1&PID_2D0[01]'
            } | Select-Object FriendlyName,Status,Class,@{Name='UsbInterface';Expression={
                if ($_.InstanceId -match '(VID_18D1&PID_2D0[01](?:&MI_[0-9A-F]{2})?)') { $matches[1] } else { 'AOA detected' }
            }} | Format-List
        }
        Bulk { Write-Host ([AnPodAoaPhase2]::BulkEcho($Rounds)) }
    }
} catch {
    Write-Host ('ERROR: ' + $_.Exception.Message) -ForegroundColor Red
    Write-Host 'Do not change the original ADB driver. Disconnect/reconnect the USB cable if you need to recover standard USB mode.'
    exit 1
}
