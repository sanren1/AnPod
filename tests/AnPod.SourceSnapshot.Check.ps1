$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $root 'packaging/source-snapshot.ps1')
$temporary = Join-Path $root '.tmp-source-snapshot-check'
if (Test-Path -LiteralPath $temporary) { throw 'Source snapshot check directory already exists.' }
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    & git -C $temporary init --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Could not initialize isolated source check.' }
    & git -C $temporary config user.email 'snapshot-check@example.invalid'
    if ($LASTEXITCODE -ne 0) { throw 'Could not configure isolated source check.' }
    & git -C $temporary config user.name 'Source Snapshot Check'
    if ($LASTEXITCODE -ne 0) { throw 'Could not configure isolated source check.' }

    $receiver = Join-Path $temporary 'src/AnPod.Receiver/app'
    $generated = Join-Path $receiver '.cxx/Debug/arm64-v8a'
    New-Item -ItemType Directory -Path $generated -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $receiver 'Tracked.java') -Value 'class Tracked {}'
    Set-Content -LiteralPath (Join-Path $receiver '.cxx/Tracked.cmake') -Value 'tracked generated-path file'
    $paths = @('src/AnPod.Receiver')
    & git -C $temporary add -- src/AnPod.Receiver
    if ($LASTEXITCODE -ne 0) { throw 'Could not stage isolated source check.' }
    & git -C $temporary commit --quiet -m 'baseline'
    if ($LASTEXITCODE -ne 0) { throw 'Could not create isolated source baseline.' }

    $baseline = Get-AnPodSourceSnapshot $temporary $paths
    if ($baseline -notmatch '^[0-9a-f]{64}$' -or (Test-AnPodSourceModified $temporary $paths)) { throw 'Clean committed source was reported modified.' }
    Set-Content -LiteralPath (Join-Path $generated 'CMakeCache.txt') -Value 'local cache'
    if ((Get-AnPodSourceSnapshot $temporary $paths) -ne $baseline -or (Test-AnPodSourceModified $temporary $paths)) { throw 'Untracked .cxx cache changed source identity or dirty state.' }

    Set-Content -LiteralPath (Join-Path $receiver 'New.java') -Value 'class NewSource {}'
    $withNewSource = Get-AnPodSourceSnapshot $temporary $paths
    if ($withNewSource -eq $baseline -or -not (Test-AnPodSourceModified $temporary $paths)) { throw 'Untracked source file was not detected.' }
    Remove-Item -LiteralPath (Join-Path $receiver 'New.java')

    Set-Content -LiteralPath (Join-Path $receiver 'Tracked.java') -Value 'class Tracked { int changed; }'
    $modifiedTracked = Get-AnPodSourceSnapshot $temporary $paths
    if ($modifiedTracked -eq $baseline -or -not (Test-AnPodSourceModified $temporary $paths)) { throw 'Tracked source modification was not detected.' }
    Set-Content -LiteralPath (Join-Path $receiver 'Tracked.java') -Value 'class Tracked {}'

    Remove-Item -LiteralPath (Join-Path $receiver '.cxx/Tracked.cmake')
    $deletedTracked = Get-AnPodSourceSnapshot $temporary $paths
    if ($deletedTracked -eq $baseline -or -not (Test-AnPodSourceModified $temporary $paths)) { throw 'Tracked generated-path deletion was not detected.' }
    Write-Output 'SOURCE_SNAPSHOT=PASS'
} finally {
    $resolved = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $temporary).Path)
    if ($resolved -ne [IO.Path]::GetFullPath((Join-Path $root '.tmp-source-snapshot-check'))) { throw 'Refusing unexpected cleanup path.' }
    if ((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing redirected cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
