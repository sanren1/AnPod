$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$publishScript = Join-Path $root 'packaging/publish-tauri.ps1'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($publishScript, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
$declarations = @($ast.FindAll({
    param($node)
    ($node -is [System.Management.Automation.Language.AssignmentStatementAst] -and
        $node.Left -is [System.Management.Automation.Language.VariableExpressionAst] -and
        $node.Left.VariablePath.UserPath -eq 'componentSourceBySha256') -or
    ($node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq 'Get-AnPodComponentBuildMetadata')
}, $false))
if ($declarations.Count -ne 2) { throw 'Could not locate the component provenance rules in publish-tauri.ps1.' }
. ([scriptblock]::Create(($declarations.Extent.Text -join "`n")))

function Assert-Rejected {
    param([scriptblock]$Action, [string]$Message)
    $rejected = $false
    try { & $Action } catch { $rejected = $true }
    if (-not $rejected) { throw $Message }
}

$temporary = Join-Path $root ('.tmp-package-components-check-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    $nativeSource = Join-Path $root 'deps/audio_capture.dll'
    $legacySource = Join-Path $root 'deps/server'
    $native = Get-AnPodComponentBuildMetadata -Path $nativeSource -Component Native
    $legacy = Get-AnPodComponentBuildMetadata -Path $legacySource -Component Legacy
    if ($native.sourceCommit -ne '393628dd2a43a221154b8226d70c03de2629cb66' -or
        $legacy.sourceCommit -ne '52795df94699204b7a3212803487c392217f95c3') {
        throw 'Production Native/Legacy provenance did not resolve to the audited source commits.'
    }

    foreach ($component in @(
        @{ Name = 'Native'; Source = $nativeSource; Metadata = $native; File = 'audio_capture.dll' },
        @{ Name = 'Legacy'; Source = $legacySource; Metadata = $legacy; File = 'server' }
    )) {
        $copy = Join-Path $temporary $component.File
        Copy-Item -LiteralPath $component.Source -Destination $copy
        $copied = Get-AnPodComponentBuildMetadata -Path $copy -Component $component.Name -ExpectedSha256 $component.Metadata.sha256
        if ($copied.sha256 -ne $component.Metadata.sha256) { throw "$($component.Name) copied bytes changed." }
    }

    $unknownNative = Join-Path $temporary 'unknown.dll'
    $unknownLegacy = Join-Path $temporary 'unknown-server'
    [IO.File]::WriteAllBytes($unknownNative, [byte[]](1, 2, 3, 4))
    [IO.File]::WriteAllBytes($unknownLegacy, [byte[]](5, 6, 7, 8))
    Assert-Rejected { Get-AnPodComponentBuildMetadata -Path $unknownNative -Component Native } 'Unknown Native DLL was accepted.'
    Assert-Rejected { Get-AnPodComponentBuildMetadata -Path $unknownLegacy -Component Legacy } 'Unknown Legacy server was accepted.'
    Assert-Rejected { Get-AnPodComponentBuildMetadata -Path $nativeSource -Component Legacy } 'Native hash was accepted as Legacy.'

    $oldPackage = Join-Path $root 'artifacts/AnPod-1.10.6-tauri-win-x64-portable'
    if (Test-Path -LiteralPath $oldPackage) {
        Assert-Rejected { Get-AnPodComponentBuildMetadata -Path (Join-Path $oldPackage 'audio_capture.dll') -Component Native } '1.10.6 Native DLL was accepted.'
        Assert-Rejected { Get-AnPodComponentBuildMetadata -Path (Join-Path $oldPackage 'server') -Component Legacy } '1.10.6 Legacy server was accepted.'
    }
    Write-Output 'PACKAGE_COMPONENTS_CHECK=PASS'
} finally {
    $resolved = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $temporary).Path)
    if (-not $resolved.StartsWith([IO.Path]::GetFullPath($root).TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing cleanup outside the workspace.'
    }
    if ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing redirected cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
