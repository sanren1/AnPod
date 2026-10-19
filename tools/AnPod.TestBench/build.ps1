$ErrorActionPreference = 'Stop'
$toolRoot = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $toolRoot '..\..')).Path
$artifactRoot = Join-Path $repoRoot 'artifacts'
$packageName = 'AnPod-TestBench-0.2.0'
$package = Join-Path $artifactRoot $packageName
$zip = Join-Path $artifactRoot ($packageName + '.zip')
if (Test-Path -LiteralPath $package) { throw "Refusing to overwrite existing package: $package" }
if (Test-Path -LiteralPath $zip) { throw "Refusing to overwrite existing archive: $zip" }
$repoFull = [IO.Path]::GetFullPath($repoRoot).TrimEnd('\') + '\'
$artifactFull = [IO.Path]::GetFullPath($artifactRoot)
if (-not $artifactFull.StartsWith($repoFull, [StringComparison]::OrdinalIgnoreCase)) { throw 'Artifact directory escaped the repository.' }
if (Test-Path -LiteralPath $artifactRoot) {
    if ((Get-Item -LiteralPath $artifactRoot -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Artifact directory is a reparse point.' }
}

function Assert-OwnedPlainPath([string]$target, [string]$root) {
    $fullRoot = [IO.Path]::GetFullPath($root).TrimEnd('\')
    $fullTarget = [IO.Path]::GetFullPath($target)
    if (-not $fullTarget.StartsWith(($fullRoot + '\'), [StringComparison]::OrdinalIgnoreCase)) { throw "Refusing cleanup outside artifacts: $fullTarget" }
    foreach ($item in @(Get-Item -LiteralPath $fullTarget -Force) + @(Get-ChildItem -LiteralPath $fullTarget -Force -Recurse -ErrorAction Stop)) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing recursive cleanup through reparse point: $($item.FullName)" }
    }
}

$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path -LiteralPath $csc)) { throw '.NET Framework 4.x C# compiler was not found.' }
$installer = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$devCmd = $null
if (Test-Path -LiteralPath $installer) {
    $vs = & $installer -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($vs) { $candidate = Join-Path $vs 'Common7\Tools\VsDevCmd.bat'; if (Test-Path -LiteralPath $candidate) { $devCmd = $candidate } }
}
if (-not $devCmd) {
    $candidate = 'E:\Software\VisualStudio\Common7\Tools\VsDevCmd.bat'
    if (Test-Path -LiteralPath $candidate) { $devCmd = $candidate }
}
if (-not $devCmd) { throw 'Visual Studio C++ build tools (VsDevCmd.bat) were not found.' }

New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null
$temp = Join-Path $artifactRoot ('.' + $packageName + '-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temp | Out-Null
$packageCreated = $false
$zipCreated = $false
try {
    $native = Join-Path $temp 'capture_probe.exe'
    $cpp = Join-Path $toolRoot 'native\capture_probe.cpp'
    $cmdFile = Join-Path $temp 'build-native.cmd'
    @(
        ('call "' + $devCmd + '" -arch=x64'),
        ('cl /nologo /EHsc /std:c++14 /O2 /DUNICODE /D_UNICODE /Fo:"' + (Join-Path $temp 'capture_probe.obj') + '" /Fd:"' + (Join-Path $temp 'capture_probe.pdb') + '" /Fe:"' + $native + '" "' + $cpp + '" /link ole32.lib uuid.lib winmm.lib'),
        'exit /b %ERRORLEVEL%'
    ) | Set-Content -LiteralPath $cmdFile -Encoding ASCII
    & $env:ComSpec /d /c "`"$cmdFile`""
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $native)) { throw 'WASAPI native build failed.' }

    $sources = @(
        (Join-Path $toolRoot 'Program.cs'),
        (Join-Path $toolRoot 'MainForm.cs'),
        (Join-Path $toolRoot 'BenchCore.cs'),
        (Join-Path $toolRoot 'Measurement.cs'),
        (Join-Path $toolRoot 'MeasurementChecks.cs')
    )
    $app = Join-Path $temp 'AnPod.TestBench.exe'
    & $csc /nologo /target:winexe /platform:x64 /out:$app /reference:System.Windows.Forms.dll /reference:System.Drawing.dll /reference:System.Web.Extensions.dll /reference:System.Numerics.dll $sources
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $app)) { throw 'WinForms C# build failed.' }
    New-Item -ItemType Directory -Path $package | Out-Null
    $packageCreated = $true
    Copy-Item -LiteralPath $app -Destination $package
    Copy-Item -LiteralPath (Join-Path $temp 'capture_probe.exe') -Destination $package
    Copy-Item -LiteralPath (Join-Path $toolRoot 'Start-AnPod-TestBench.cmd') -Destination $package
    Copy-Item -LiteralPath (Join-Path $toolRoot 'README.md') -Destination $package
    $zipCreated = $true
    Compress-Archive -Path (Join-Path $package '*') -DestinationPath $zip -CompressionLevel Optimal
    Write-Output "Package: $package"
    Write-Output "Archive: $zip"
}
catch {
    if ($packageCreated -and (Test-Path -LiteralPath $package)) { Assert-OwnedPlainPath $package $artifactRoot; Remove-Item -LiteralPath $package -Recurse -Force }
    if ($zipCreated -and (Test-Path -LiteralPath $zip)) { Assert-OwnedPlainPath $zip $artifactRoot; Remove-Item -LiteralPath $zip -Force }
    throw
}
finally {
    if (Test-Path -LiteralPath $temp) { Assert-OwnedPlainPath $temp $artifactRoot; Remove-Item -LiteralPath $temp -Recurse -Force }
}
