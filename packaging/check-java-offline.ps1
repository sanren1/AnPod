$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$javac = if ($env:JAVA_HOME) { Join-Path $env:JAVA_HOME 'bin\javac.exe' } else { (Get-Command javac -ErrorAction Stop).Source }
$java = if ($env:JAVA_HOME) { Join-Path $env:JAVA_HOME 'bin\java.exe' } else { (Get-Command java -ErrorAction Stop).Source }
$tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
$classes = [IO.Path]::GetFullPath((Join-Path $tempRoot "anpod-java-offline-$PID"))
if (-not $classes.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Temporary Java output must stay under TEMP.'
}
$sources = @(
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\PairingProtocol.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\P5cControlProtocol.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\UdpSessionContext.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\UdpPacketDecoder.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\UdpPcmQueue.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\UdpReceiver.java',
    'src\AnPod.AndroidProtocol\src\main\java\com\anpod\server\Pcm16FrameStream.java',
    'src\AnPod.AndroidServer\src\main\java\com\anpod\server\Options.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\AudioFocusState.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\BackgroundStartAuthorization.java',
    'src\AnPod.Receiver\app\src\main\java\app\anpod\receiver\DiscoveryAuth.java',
    'tests\AnPodUdpVectorCheck.java',
    'tests\AnPod.AndroidServer.PcmCheck.java',
    'tests\AnPodAudioFocusStateCheck.java',
    'tests\BackgroundStartAuthorizationCheck.java',
    'tests\AnPodDiscoveryAuthCheck.java'
) | ForEach-Object { Join-Path $repo $_ }

New-Item -ItemType Directory -Path $classes | Out-Null
try {
    & $javac --release 17 -encoding UTF-8 -d $classes @sources
    if ($LASTEXITCODE -ne 0) { throw "javac failed with exit code $LASTEXITCODE" }

    & $java -cp $classes app.anpod.receiver.AnPodDiscoveryAuthCheck
    if ($LASTEXITCODE -ne 0) { throw "AnPodDiscoveryAuthCheck failed with exit code $LASTEXITCODE" }

    & $java -cp $classes app.anpod.receiver.AnPodUdpVectorCheck
    if ($LASTEXITCODE -ne 0) { throw "AnPodUdpVectorCheck failed with exit code $LASTEXITCODE" }

    & $java -cp $classes com.anpod.server.PcmCheck
    if ($LASTEXITCODE -ne 0) { throw "PcmCheck failed with exit code $LASTEXITCODE" }

    & $java -cp $classes app.anpod.receiver.AnPodAudioFocusStateCheck
    if ($LASTEXITCODE -ne 0) { throw "AnPodAudioFocusStateCheck failed with exit code $LASTEXITCODE" }

    & $java -cp $classes app.anpod.receiver.BackgroundStartAuthorizationCheck
    if ($LASTEXITCODE -ne 0) { throw "BackgroundStartAuthorizationCheck failed with exit code $LASTEXITCODE" }
}
finally {
    Remove-Item -LiteralPath $classes -Recurse -Force
}
