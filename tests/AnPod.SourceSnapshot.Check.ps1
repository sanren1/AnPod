$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'packaging/source-snapshot.ps1')
$temporary = Join-Path $root '.tmp-source-snapshot-check'
if (Test-Path -LiteralPath $temporary) { throw 'Source snapshot check directory already exists.' }
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    & git -C $temporary init --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Could not initialize isolated source check.' }
    Set-Content -LiteralPath (Join-Path $temporary 'input.txt') -Value 'first'
    Set-Content -LiteralPath (Join-Path $temporary 'private.txt') -Value 'excluded'
    $first = Get-AnPodSourceSnapshot $temporary @('input.txt')
    if ($first -notmatch '^[0-9a-f]{64}$' -or $first -ne (Get-AnPodSourceSnapshot $temporary @('input.txt'))) { throw 'Unchanged source digest is unstable.' }
    Set-Content -LiteralPath (Join-Path $temporary 'private.txt') -Value 'still excluded'
    if ($first -ne (Get-AnPodSourceSnapshot $temporary @('input.txt'))) { throw 'Unselected private file entered source digest.' }
    Set-Content -LiteralPath (Join-Path $temporary 'input.txt') -Value 'changed'
    if ($first -eq (Get-AnPodSourceSnapshot $temporary @('input.txt'))) { throw 'Source mutation was not detected.' }
    Write-Output 'SOURCE_SNAPSHOT=PASS'
} finally {
    $resolved = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $temporary).Path)
    if ($resolved -ne [IO.Path]::GetFullPath((Join-Path $root '.tmp-source-snapshot-check'))) { throw 'Refusing unexpected cleanup path.' }
    if ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing redirected cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
