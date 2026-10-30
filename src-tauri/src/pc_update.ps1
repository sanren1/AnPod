param(
  [Parameter(Mandatory=$true)][ValidateSet('check','download','install')][string]$Mode,
  [string]$ZipUrl,
  [string]$SumsUrl,
  [string]$AssetName,
  [long]$AssetSize,
  [long]$SumsSize,
  [string]$StageDirectory,
  [string]$TargetDirectory,
  [string]$ExeName,
  [string]$ExePath,
  [string]$ResultPath,
  [string]$MutexName,
  [string]$ReadyPath,
  [string]$ContinuePath,
  [int]$ProcessId,
  [long]$ProcessStartTime,
  [switch]$SkipWait,
  [switch]$SilentTest
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [Text.UTF8Encoding]::new($false)
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$script:ParentProcess = $null
$script:InstanceMutex = $null

function Get-VerifiedParentProcess {
  try { $process = [Diagnostics.Process]::GetProcessById($ProcessId) }
  catch [ArgumentException] { return $null }
  $processPath = Normalize-ProcessPath $process.MainModule.FileName
  $expectedPath = Normalize-ProcessPath $ExePath
  if (-not $processPath.Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
    $process.Dispose()
    throw '待退出进程的程序路径与当前AnPod不一致，拒绝替换。'
  }
  if ($process.StartTime.ToFileTimeUtc() -ne $ProcessStartTime) {
    $process.Dispose()
    throw '待退出进程的启动时间与当前AnPod不一致，拒绝替换。'
  }
  return $process
}

function Normalize-ProcessPath([string]$Path) {
  $full = [IO.Path]::GetFullPath($Path)
  if ($full.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) { return '\\' + $full.Substring(8) }
  if ($full.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) { return $full.Substring(4) }
  return $full
}

trap {
  $message = "$($_.Exception.Message) (pc_update.ps1:$($_.InvocationInfo.ScriptLineNumber))"
  if ($Mode -eq 'install') {
    if (-not $SkipWait -and $ReadyPath -and -not [IO.File]::Exists($ReadyPath)) {
      if ($message.Length -gt 512) { $message = $message.Substring(0, 512) }
      try { Write-Handoff $ReadyPath ('error:' + $message) }
      catch { $message += "；无法通知AnPod预检失败：$($_.Exception.Message)" }
    }
    $preflightFailed = $false
    if (-not $SkipWait -and $ReadyPath -and [IO.File]::Exists($ReadyPath)) {
      try { $preflightFailed = [IO.File]::ReadAllText($ReadyPath).StartsWith('error:') }
      catch { $message += "；无法读取交接状态：$($_.Exception.Message)" }
    }
    if (-not $SkipWait -and -not $preflightFailed) {
      $deadline = [DateTime]::UtcNow.AddSeconds(20)
      try { if ($null -eq $script:ParentProcess) { $script:ParentProcess = Get-VerifiedParentProcess }; if ($script:ParentProcess -and -not $script:ParentProcess.HasExited) { [void]$script:ParentProcess.WaitForExit([int]([Math]::Max(0, ($deadline - [DateTime]::UtcNow).TotalMilliseconds))) } } catch { $message += "；等待原程序退出失败：$($_.Exception.Message)" }
    }
    $resultStatus = if ($script:InstallResultStatus) { $script:InstallResultStatus } else { 'failed' }
    try { Write-UpdateResult $ResultPath $resultStatus $message }
    catch { $message += "；保存更新结果失败：$($_.Exception.Message)" }
    if ($resultStatus -eq 'rollbackFailed') {
      $message += '；自动启动已停止，请保留备份目录并手动恢复后再启动。'
    } elseif (-not $SkipWait -and -not $preflightFailed -and ($null -eq $script:ParentProcess -or $script:ParentProcess.HasExited)) {
      if ($script:InstanceMutex) { $script:InstanceMutex.Dispose(); $script:InstanceMutex = $null }
      try { Start-Process -FilePath (Join-Path $TargetDirectory $ExeName) -WorkingDirectory $TargetDirectory | Out-Null }
      catch { $message += "；重新启动原程序失败：$($_.Exception.Message)"; try { Write-UpdateResult $ResultPath $resultStatus $message } catch { [Console]::Error.WriteLine($_.Exception.Message) } }
    }
    if ($script:InstanceMutex) { $script:InstanceMutex.Dispose(); $script:InstanceMutex = $null }
    if (-not $SilentTest) {
      Show-UpdateFailure $message
    }
  }
  [Console]::Error.WriteLine($message)
  exit 1
}

function Show-UpdateFailure([string]$Message) {
  try {
    Add-Type -AssemblyName System.Windows.Forms
    [void][System.Windows.Forms.MessageBox]::Show("AnPod更新失败：$Message", 'AnPod 更新失败', 'OK', 'Error')
  } catch {
    try { (New-Object -ComObject WScript.Shell).Popup("AnPod更新失败：$Message", 0, 'AnPod 更新失败', 16) | Out-Null }
    catch { [Console]::Error.WriteLine("无法显示更新错误：$($_.Exception.Message)；$Message") }
  }
}

function Write-Handoff([string]$Path, [string]$Status) {
  $temporary = $Path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
  try {
    [IO.File]::WriteAllText($temporary, $Status, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($temporary, $Path)
  } finally {
    if ([IO.File]::Exists($temporary)) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
  }
}

function Write-UpdateResult([string]$Path, [string]$Status, [string]$Message) {
  if ($Message.Length -gt 512) { $Message = $Message.Substring(0, 512) }
  $parent = Split-Path -Parent $Path
  Assert-NoReparse $parent
  if ([IO.File]::Exists($Path) -and ((Get-Item -LiteralPath $Path).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '更新结果文件是重解析点。' }
  $body = [pscustomobject]@{ status=$Status; message=$Message } | ConvertTo-Json -Compress
  $temporary = $Path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
  $previous = $Path + '.' + [Guid]::NewGuid().ToString('N') + '.old'
  try {
    [IO.File]::WriteAllText($temporary, $body, [Text.UTF8Encoding]::new($false))
    if ([IO.File]::Exists($Path)) {
      [IO.File]::Replace($temporary, $Path, $previous)
      Remove-Item -LiteralPath $previous -Force
    } else { [IO.File]::Move($temporary, $Path) }
  } finally {
    if ([IO.File]::Exists($temporary)) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
    if ([IO.File]::Exists($previous)) { Remove-Item -LiteralPath $previous -Force -ErrorAction SilentlyContinue }
  }
}

function Assert-NoReparse([string]$Path) {
  $full = [IO.Path]::GetFullPath($Path)
  $current = [IO.DirectoryInfo]::new($full)
  while ($null -ne $current) {
    if ($current.Exists -and (($current.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
      throw "拒绝经过重解析点的路径：$($current.FullName)"
    }
    $current = $current.Parent
  }
}

function Get-SafeEntries([string]$ArchivePath) {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $archive = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
  try {
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $entries = @()
    [long]$expandedBytes = 0
    foreach ($entry in $archive.Entries) {
      $name = $entry.FullName
      if ([string]::IsNullOrWhiteSpace($name) -or $name.StartsWith('/') -or $name.StartsWith('\') -or $name.Contains('\') -or $name.Contains(':')) { throw "ZIP路径无效：$name" }
      $parts = $name.TrimEnd('/').Split('/')
      $invalidParts = @($parts | Where-Object { $_ -eq '' -or $_ -eq '.' -or $_ -eq '..' -or $_.EndsWith('.') -or $_.EndsWith(' ') })
      if ($parts.Count -eq 0 -or $invalidParts.Count -gt 0) { throw "ZIP路径越界或无效：$name" }
      foreach ($part in $parts) { if ($part -match '^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') { throw "ZIP包含Windows保留名称：$name" } }
      Assert-ManagedName -RelativePath $name
      $unixType = ($entry.ExternalAttributes -shr 16) -band 0xF000
      if ($unixType -eq 0xA000) { throw "ZIP不允许符号链接：$name" }
      $key = $name.TrimEnd('/')
      if ($key -and -not $seen.Add($key)) { throw "ZIP包含重复路径：$name" }
      if (-not $name.EndsWith('/')) {
        $expandedBytes += [long]$entry.Length
        if ($expandedBytes -gt 1073741824) { throw 'ZIP解压后超过1 GiB，已拒绝。' }
        $entries += [pscustomobject]@{ Name=$name; Length=$entry.Length }
      }
    }
    if ($entries.Count -eq 0) { throw 'ZIP不包含可安装文件。' }
    return $entries
  } finally { $archive.Dispose() }
}

function Download-Asset([string]$Url, [string]$Path, [long]$ExpectedSize, [long]$MaximumSize) {
  if ($ExpectedSize -le 0 -or $ExpectedSize -gt $MaximumSize) { throw 'Release资产大小无效。' }
  $request = [Net.HttpWebRequest]::Create($Url)
  $request.Method = 'GET'
  $request.UserAgent = 'AnPod-Updater'
  $request.Timeout = 90000
  $request.ReadWriteTimeout = 90000
  $request.AllowAutoRedirect = $true
  $response = $null
  $sourceStream = $null
  $targetStream = $null
  try {
    $response = $request.GetResponse()
    if ($response.ResponseUri.Scheme -ne 'https') { throw '下载重定向不安全。' }
    if ($response.ContentLength -ne $ExpectedSize) { throw '下载资产的HTTP长度与Release记录不一致。' }
    $sourceStream = $response.GetResponseStream()
    $targetStream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    [byte[]]$buffer = New-Object byte[] 65536
    [long]$total = 0
    while (($read = $sourceStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
      $total += $read
      if ($total -gt $ExpectedSize -or $total -gt $MaximumSize) { throw '下载资产超出Release记录大小。' }
      $targetStream.Write($buffer, 0, $read)
    }
    if ($total -ne $ExpectedSize) { throw '下载资产长度与Release记录不一致。' }
  } catch {
    if ([IO.File]::Exists($Path)) { Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue }
    throw
  } finally {
    if ($targetStream) { $targetStream.Dispose() }
    if ($sourceStream) { $sourceStream.Dispose() }
    if ($response) { $response.Dispose() }
  }
}

function Get-Sha256([string]$Path) {
  $stream = [IO.File]::OpenRead($Path)
  $sha = [Security.Cryptography.SHA256]::Create()
  try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
  finally { $sha.Dispose(); $stream.Dispose() }
}

function Assert-ManagedName([string]$RelativePath) {
  $parts = $RelativePath.TrimEnd('/').Split('/')
  $rootFiles = @('AnPod.exe','audio_capture.dll','adb.exe','AdbWinApi.dll','AdbWinUsbApi.dll','server','receiver.apk','receiver.json','build-metadata.json','icon.png','README.md','THIRD_PARTY_NOTICES.md','LICENSE')
  if ($parts.Count -eq 1 -and $rootFiles -ccontains $parts[0]) { return }
  if ($parts.Count -gt 1 -and @('docs','LICENSES','packaging','upstream','USB-Direct') -ccontains $parts[0]) { return }
  throw "ZIP包含非AnPod托管路径，拒绝自动覆盖用户文件：$RelativePath"
}

if ($Mode -eq 'check') {
  $release = Invoke-RestMethod -Method Get -Uri 'https://api.github.com/repos/sanren1/AnPod/releases/latest' -Headers @{ 'User-Agent'='AnPod-Updater'; 'Accept'='application/vnd.github+json' } -TimeoutSec 30
  if ($release.draft -or $release.prerelease -or $release.tag_name -notmatch '^v(\d+\.\d+\.\d+)-main$') { throw 'GitHub返回的latest不是AnPod正式main版本。' }
  $version = $Matches[1]
  $expected = "AnPod-$version-tauri-win-x64-portable.zip"
  $zip = @($release.assets | Where-Object { $_.name -ceq $expected })
  $sums = @($release.assets | Where-Object { $_.name -ceq 'SHA256SUMS.txt' })
  if ($zip.Count -ne 1 -or $sums.Count -ne 1 -or $zip[0].size -le 0 -or $zip[0].size -gt 524288000 -or $sums[0].size -le 0 -or $sums[0].size -gt 1048576) { throw '正式Release缺少唯一便携ZIP或SHA256SUMS.txt，或资产大小超出限制。' }
  $releaseUrl = [string]$release.html_url
  $zipUri = [Uri]$zip[0].browser_download_url
  $sumsUri = [Uri]$sums[0].browser_download_url
  if ($releaseUrl -cne "https://github.com/sanren1/AnPod/releases/tag/$($release.tag_name)" -or $zipUri.Scheme -ne 'https' -or $zipUri.Host -ne 'github.com' -or $sumsUri.Scheme -ne 'https' -or $sumsUri.Host -ne 'github.com') { throw 'Release URL不符合官方仓库约定。' }
  [pscustomobject]@{ version=$version; releaseUrl=$releaseUrl; assetName=$expected; assetSize=[long]$zip[0].size; sumsSize=[long]$sums[0].size; zipUrl=[string]$zip[0].browser_download_url; sumsUrl=[string]$sums[0].browser_download_url } | ConvertTo-Json -Compress
  exit 0
}

if ($Mode -eq 'download') {
  Assert-NoReparse $StageDirectory
  [void][IO.Directory]::CreateDirectory($StageDirectory)
  $stage = [IO.Path]::GetFullPath($StageDirectory)
  $zipPath = Join-Path $stage $AssetName
  $sumsPath = Join-Path $stage 'SHA256SUMS.txt'
  Download-Asset $ZipUrl $zipPath $AssetSize 524288000
  Download-Asset $SumsUrl $sumsPath $SumsSize 1048576
  $line = @(Get-Content -LiteralPath $sumsPath | Where-Object { $_ -match '^([0-9a-fA-F]{64})  (.+)$' -and $Matches[2] -ceq $AssetName })
  if ($line.Count -ne 1) { throw 'SHA256SUMS.txt中没有唯一的ZIP摘要行。' }
  $expectedHash = [regex]::Match($line[0], '^([0-9a-fA-F]{64})  ').Groups[1].Value.ToLowerInvariant()
  $actualHash = Get-Sha256 $zipPath
  if ($actualHash -cne $expectedHash) { throw '下载ZIP的SHA-256校验失败。' }
  $entries = Get-SafeEntries $zipPath
  $package = Join-Path $stage 'package'
  [void][IO.Directory]::CreateDirectory($package)
  Assert-NoReparse $package
  $archive = [IO.Compression.ZipFile]::OpenRead($zipPath)
  try {
    foreach ($entry in $archive.Entries) {
      if ($entry.FullName.EndsWith('/')) { continue }
      $destination = [IO.Path]::GetFullPath((Join-Path $package ($entry.FullName -replace '/', '\')))
      if (-not $destination.StartsWith($package.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "ZIP路径越界：$($entry.FullName)" }
      $parent = Split-Path -Parent $destination
      [void][IO.Directory]::CreateDirectory($parent)
      Assert-NoReparse $parent
      $input = $entry.Open()
      try { $output = [IO.File]::Open($destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None); try { $input.CopyTo($output) } finally { $output.Dispose() } } finally { $input.Dispose() }
    }
  } finally { $archive.Dispose() }
  $manifestFiles = @(foreach ($entry in $entries) {
    $path = Join-Path $package ($entry.Name -replace '/', '\')
    [pscustomobject]@{ name=$entry.Name; length=[long]$entry.Length; sha256=(Get-Sha256 $path) }
  })
  $manifest = [pscustomobject]@{ files=$manifestFiles }
  [IO.File]::WriteAllText((Join-Path $stage 'manifest.json'), (ConvertTo-Json -InputObject $manifest -Compress), [Text.UTF8Encoding]::new($false))
  [pscustomobject]@{ status='prepared' } | ConvertTo-Json -Compress
  exit 0
}

if ($Mode -eq 'install') {
  $target = [IO.Path]::GetFullPath($TargetDirectory).TrimEnd('\')
  $stage = [IO.Path]::GetFullPath($StageDirectory).TrimEnd('\')
  Assert-NoReparse $target
  Assert-NoReparse $stage
  $manifestPath = Join-Path $stage 'manifest.json'
  $manifestDocument = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($manifestPath))
  $manifest = @($manifestDocument.files)
  if ($manifest.Count -eq 0) { throw '更新文件清单为空。' }
  foreach ($item in $manifest) {
    $name = [string]$item.name
    $invalidParts = @($name.Split('/') | Where-Object { $_ -eq '' -or $_ -eq '.' -or $_ -eq '..' })
    if ($name.Contains('\') -or $name.Contains(':') -or $name.StartsWith('/') -or $invalidParts.Count -gt 0) { throw "更新清单路径无效：$name" }
    Assert-ManagedName -RelativePath $name
    $source = [IO.Path]::GetFullPath((Join-Path (Join-Path $stage 'package') ($name -replace '/', '\')))
    if (-not $source.StartsWith((Join-Path $stage 'package').TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or -not [IO.File]::Exists($source)) { throw "更新文件缺失或越界：$name" }
    if (([IO.File]::GetAttributes($source) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "更新源文件是重解析点：$name" }
    if ((Get-Item -LiteralPath $source).Length -ne [long]$item.length -or (Get-Sha256 $source) -cne [string]$item.sha256) { throw "更新文件校验失败：$name" }
  }
  if (-not $SkipWait) {
    try {
      $script:ParentProcess = Get-VerifiedParentProcess
      $script:InstanceMutex = [Threading.Mutex]::OpenExisting($MutexName)
      $loaded = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
      if ($script:ParentProcess) { foreach ($module in $script:ParentProcess.Modules) { [void]$loaded.Add([IO.Path]::GetFullPath($module.FileName)) } }
    } catch { throw "无法检查当前AnPod的文件占用：$($_.Exception.Message)" }
    foreach ($item in $manifest) {
      $destination = [IO.Path]::GetFullPath((Join-Path $target ($item.name -replace '/', '\')))
      if ([IO.File]::Exists($destination) -and ([IO.File]::GetAttributes($destination) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "安装目标文件是重解析点：$($item.name)" }
      if ([IO.File]::Exists($destination) -and (Get-Sha256 $destination) -cne [string]$item.sha256 -and -not $loaded.Contains($destination)) {
        try { $probe = [IO.File]::Open($destination, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); $probe.Dispose() }
        catch { throw "更新文件被占用或无权写入：$($item.name)；$($_.Exception.Message)" }
      }
    }
    Write-Handoff $ReadyPath 'ready'
    $continueDeadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $continueDeadline -and -not [IO.File]::Exists($ContinuePath)) { Start-Sleep -Milliseconds 50 }
    if (-not [IO.File]::Exists($ContinuePath)) { throw '更新交接确认超时，未替换文件。' }
  }
  if (-not $SkipWait) {
    if ($script:ParentProcess -and -not $script:ParentProcess.WaitForExit(90000)) { throw '等待 AnPod 正常退出超时，未替换文件。' }
  }
  $backup = Join-Path $target ('.anpod-update-backup-' + [Guid]::NewGuid().ToString('N'))
  [void][IO.Directory]::CreateDirectory($backup)
  $changed = [Collections.Generic.List[object]]::new()
  try {
    foreach ($item in $manifest) {
      $name = [string]$item.name
      $source = Join-Path (Join-Path $stage 'package') ($name -replace '/', '\')
      $destination = [IO.Path]::GetFullPath((Join-Path $target ($name -replace '/', '\')))
      if (-not $destination.StartsWith($target + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "目标路径越界：$name" }
      $parent = Split-Path -Parent $destination
      Assert-NoReparse $parent
      [void][IO.Directory]::CreateDirectory($parent)
      if ([IO.Directory]::Exists($destination)) { throw "目标文件路径已被目录占用：$name" }
      $temporary = $null
      $replaceBackup = $null
      if ([IO.File]::Exists($destination)) {
        if (([IO.File]::GetAttributes($destination) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "安装目标文件是重解析点：$name" }
        if ((Get-Sha256 $destination) -ceq [string]$item.sha256) { continue }
        $backupFile = Join-Path $backup ($name -replace '/', '\')
        [void][IO.Directory]::CreateDirectory((Split-Path -Parent $backupFile))
        [IO.File]::Copy($destination, $backupFile, $false)
        $change = [pscustomobject]@{ destination=$destination; backup=$backupFile; created=$false }
      } else { $change = [pscustomobject]@{ destination=$destination; backup=$null; created=$true } }
      $temporary = $destination + '.anpod-new-' + [Guid]::NewGuid().ToString('N')
      [IO.File]::Copy($source, $temporary, $false)
      if ($change.created) { [IO.File]::Move($temporary, $destination) }
      else {
        $replaceBackup = $change.backup + '.replace'
        [IO.File]::Replace($temporary, $destination, $replaceBackup)
      }
      $changed.Add($change)
      if ($replaceBackup) { Remove-Item -LiteralPath $replaceBackup -Force -ErrorAction SilentlyContinue }
    }
    if (-not [IO.File]::Exists((Join-Path $target $ExeName))) { throw '更新包缺少主程序。' }
    Write-UpdateResult $ResultPath 'installed' 'AnPod更新已完成。'
  } catch {
    $original = $_.Exception.Message
    if ($temporary -and [IO.File]::Exists($temporary)) { Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue }
    $rollbackErrors = @()
    for ($i = $changed.Count - 1; $i -ge 0; $i--) {
      $entry = $changed[$i]
      try {
        if ($entry.created) { if ([IO.File]::Exists($entry.destination)) { [IO.File]::Delete($entry.destination) } }
        elseif ([IO.File]::Exists($entry.backup)) { [IO.File]::Copy($entry.backup, $entry.destination, $true) }
      } catch { $rollbackErrors += $_.Exception.Message }
    }
    if ($rollbackErrors.Count -eq 0) {
      Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue
      $script:InstallResultStatus = 'rolledBack'
      throw "更新失败，已恢复原文件：$original"
    }
    $script:InstallResultStatus = 'rollbackFailed'
    throw "更新失败且回滚未完成：$original；$($rollbackErrors -join '; ')"
  }
  if ($script:InstanceMutex) { $script:InstanceMutex.Dispose(); $script:InstanceMutex = $null }
  try { Start-Process -FilePath (Join-Path $target $ExeName) -WorkingDirectory $target | Out-Null }
  catch {
    $message = "AnPod文件已更新，但重新启动失败：$($_.Exception.Message)；更新文件已保留；备份目录：$backup"
    $script:InstallResultStatus = 'failed'
    try { Write-UpdateResult $ResultPath 'failed' $message }
    catch { $message += "；无法保存结果：$($_.Exception.Message)" }
    if (-not $SilentTest) { Show-UpdateFailure $message }
    [Console]::Error.WriteLine($message)
    exit 1
  }
  Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue
  exit 0
}
