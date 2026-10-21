function Get-AnPodSourceSnapshot([string]$Root, [string[]]$Paths) {
    $files = @(& git -C $Root ls-files --cached --others --exclude-standard -- @Paths | Sort-Object -Unique -CaseSensitive)
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
