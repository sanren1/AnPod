export type Device = {
  serial: string; transport: "usb" | "wifi" | "unknown"; state: string;
  manufacturer: string; model: string; androidVersion: string; apiLevel: string;
  displayName: string; statusText: string; connectable: boolean;
};
export type AoaTarget = { id: string; label: string };
export type Settings = {
  schemaVersion: number; lastDeviceId: string | null;
  minimizeToTray: boolean; launchAtLogin: boolean; preferredTransport: "Usb" | "Wifi";
  wiredMode: "adb" | "aoa"; lastUsbDirectTargetId: string | null; lastUsbDirectBindingReference: string | null;
  wirelessProtocol: "udp" | "adb"; audioBackend: "receiver" | "legacy";
  lastWirelessEndpoint: string | null;
  lastReceiverEndpoint: string | null;
  audioQuality: "standard" | "hifi" | "hires";
  playbackMode: "Balanced" | "LowLatency"; appearanceTheme: "System" | "Light" | "Dark";
};
export type NegotiatedAudioFormat = {
  codecVersion: 1 | 2; sampleFormat: "pcm16le" | "pcm24le" | "float32le"; sampleRateHz: number;
  channels: number; validBitsPerSample: number; bytesPerSample: number; frameBytes: number;
};
export type WindowsMixFormat = {
  sampleRate: number | null; channels: number | null; sampleFormat: "integer" | "float" | null;
  containerBits: number | null; validBits: number | null;
};
export type AndroidOutputFormat = {
  sampleRate: number | null; channels: number | null; encoding: "pcm16" | "pcm24Packed" | "float32" | null;
};
export type Snapshot = {
  state: "noDevice" | "unauthorized" | "ready" | "connecting" | "connected" | "streaming" | "phoneReady" | "paused" | "reconnecting" | "managementOnly" | "error";
  message: string; deviceName: string | null; devices: Device[]; selectedSerial: string | null; sessionSerial: string | null;
  sessionReceiverBinding: string | null; sessionTransport: Device["transport"] | "lan" | "aoa" | null;
  sessionGeneration: number | null;
  sessionConnectionTransport: "usb" | "wireless" | "usbDirect" | null; sessionWirelessProtocol: "udp" | "adb" | null;
  sessionAndroidVersion: string | null; connectionTransport: "usb" | "wireless"; transport: "usb" | "wifi";
  presence: PresenceEntry[];
  sessionCapabilities: ReceiverCapabilities | null; wirelessConnectionOffers: HostDiscoverySnapshot;
  playbackDiagnostics: PlaybackDiagnostics | null;
  negotiatedAudioFormat: NegotiatedAudioFormat | null;
  windowsMixFormat: WindowsMixFormat | null; androidOutputFormat: AndroidOutputFormat | null;
  output: "computer" | "phone"; settings: Settings;
  busy: boolean; canDisconnect: boolean; canChangeModes: boolean; canChangePlaybackMode: boolean; statusDetail: string;
  audioBitrateBps: number | null; networkRecoverySecondsRemaining?: number | null; version: string;
};
export type Credentials = { instanceName: string; password: string; payload: string };
export type MdnsResults = { available: boolean; services: { name: string; endpoint: string; kind: "pairing" | "connect" }[] };
export type PrivateIpv4Interface = { index: number; name: string; address: string; prefixLength: number; gateways: string[] };
export type ReceiverIpv4Interface = { address: string; prefixLength: number; interfaceIndex: number; interfaceName: string };
export type NetworkInterfacePair = { local: PrivateIpv4Interface; receiver: ReceiverIpv4Interface };
export type ReceiverStatus = {
  installed: boolean; versionName: string | null; versionCode: number | null; serviceActive: boolean;
  foreground: boolean; healthy: boolean; audioActive: boolean; error: string | null; message: string;
};
export type NetworkCandidate = { endpoint: string; bindingReference: string | null };
export type PresenceEntry = { bindingReference: string; online: boolean; lastSeenUnix: number | null; lastAddress: string | null };
export type ReceiverCapabilities = {
  controlMajor: number; audioMajor: number; managementCompatible: boolean;
  audioCompatible: boolean; compatible: boolean; capabilityBits: number; versionCode: number;
};
export type PlaybackDiagnostics = {
  sampleRate: number | null; channels: number | null;
  receiverSampleRate: number | null; receiverChannels: number | null;
  pcDroppedFrames: number | null; pcQueuedFrames: number | null;
  udpJitterTargetFrames: number | null; udpExpiredFrames: number | null; udpQueuedFrames: number | null;
  tcpDroppedBacklogFrames: number | null; trackPendingFrames: number | null;
  underrunCount: number | null; underrunIncrement: number | null; playbackAdvancing: boolean | null;
  requestedBufferFrames: number | null; acceptedBufferFrames: number | null; effectiveBufferFrames: number | null;
  capacityFrames: number | null; startThresholdFrames: number | null; preRollFrames: number | null;
};
export type ReceiverPackageCandidate = { versionCode: number; versionName: string; sizeBytes: number; currentVersionCode: number | null; bindingReference: string | null };
export type BindingResetPreview = { serial: string; androidUser: string; bindingReference: string; phoneState: string; phoneCommitment: string | null; pcState: string; pcCommitment: string | null; receiverBusy: boolean };
export type BindingResetResult = Omit<BindingResetPreview, "receiverBusy"> & { outcome: string; reason: string; nextAction: string };
export type ReceiverUsbOperation = { kind: "install" | "upgrade"; serial: string; bindingReference?: string; targetVersionCode: number; returnPanel: "network" | "receiver" | "setup" };
export type ReceiverUsbOperationResult = { serial: string; kind: ReceiverUsbOperation["kind"]; targetVersionCode: number; status: ReceiverStatus };
export type ReceiverInstallProgress = { operationId: string; serial: string; stage: "checking" | "transferring" | "verifying" | "completed" | "failed" };
export type ConnectionOffer = { id: string; endpoint: string };
export type HostDiscoverySnapshot = {
  active: boolean; automaticBinding?: string | null; interfaceAddress: string | null; interfacePrefixLength: number | null; offers: ConnectionOffer[];
};
export type NetworkStartRequest = {
  localAddress: string; peer: string; bootstrapSerial?: string; receiverAddress?: string;
  receiverPrefixLength?: number; bindingReference?: string; backgroundAuthorizationRequired?: boolean;
};
export type BackgroundTrustTarget =
  | { target: "activeSession"; bindingReference: string; sessionGeneration: number }
  | { target: "receiver"; bindingReference: string; localAddress: string; peer: string };
export type BackgroundTrustStatus = {
  target: "activeSession" | "receiver"; bindingReference: string; sessionGeneration: number | null;
  kind: "none" | "once" | "eightHours" | "untilRevoked";
  persistence: "none" | "process" | "binding"; remainingMs: number | null;
};
export type OperationResult = { success: boolean; message: string; snapshot: Snapshot };
