function Get-AnPodUntrackedSourceFiles([string]$Root, [string[]]$Paths) {
    $files = @(& git -C $Root ls-files --others --exclude-standard -- @Paths)
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate untracked source inputs.' }
    return @($files | Where-Object {
        $path = $_ -replace '\\', '/'
        -not $path.StartsWith('src/AnPod.Receiver/app/.cxx/', [StringComparison]::Ordinal)
    })
}

function Get-AnPodSourceFiles([string]$Root, [string[]]$Paths) {
    $tracked = @(& git -C $Root ls-files --cached -- @Paths)
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate tracked source inputs.' }
    $untracked = @(Get-AnPodUntrackedSourceFiles $Root $Paths)
    $files = @(@($tracked) + @($untracked) | Sort-Object -Unique -CaseSensitive)
    return $files
}

function Test-AnPodSourceModified([string]$Root, [string[]]$Paths) {
    $trackedChanges = @(& git -C $Root diff --name-only HEAD -- @Paths)
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect tracked source changes.' }
    $untrackedChanges = @(Get-AnPodUntrackedSourceFiles $Root $Paths)
    return ($trackedChanges.Count -gt 0 -or $untrackedChanges.Count -gt 0)
}

function Get-AnPodSourceSnapshot([string]$Root, [string[]]$Paths) {
    $files = @(Get-AnPodSourceFiles $Root $Paths)
    if ($LASTEXITCODE -ne 0 -or -not $files.Count) { throw 'Could not enumerate candidate source inputs.' }
    $entries = foreach ($relative in $files) {
        $path = Join-Path $Root $relative
        $digest = if (Test-Path -LiteralPath $path -PathType Leaf) {
            (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        } else { 'deleted' }
        "$relative=$digest"
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes(($entries -join "`n"))
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
