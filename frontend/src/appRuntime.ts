import { invoke } from "@tauri-apps/api/core";
import { strings } from "./strings";
import type { Device, Snapshot, PrivateIpv4Interface, ReceiverIpv4Interface, NetworkInterfacePair, BackgroundTrustStatus } from "./appTypes";

export const isDesktop = "__TAURI_INTERNALS__" in window;
export const invokeApp = <T,>(command: string, args?: Record<string, unknown>) => {
  if (!isDesktop) return Promise.reject(new Error(strings.app.desktopOnly));
  return invoke<T>(command, args);
};
export const deviceOsLabel = (device: Device) => device.platform === "harmony" ? device.osVersion : device.androidVersion ? `Android ${device.androidVersion}` : "";
// A HarmonyOS tablet connects over hdc only; the saved Android wired mode stays as it is.
export const isHarmony = (device?: Device) => device?.platform === "harmony";
export const wiredModeChoice = (saved: Snapshot["settings"]["wiredMode"], device?: Device) =>
  isHarmony(device) ? { value: "hdc" as const, options: ["hdc" as const] } : { value: saved, options: ["adb", "aoa"] as const };
export const effectiveWiredMode = (saved: Snapshot["settings"]["wiredMode"], device?: Device) => isHarmony(device) ? "adb" : saved;
// With a tablet attached the picker lists devices even in USB Direct mode, so the tablet stays choosable.
export const pickerOpensUsbDirect = (snapshot: Snapshot, wiredMode: Snapshot["settings"]["wiredMode"]) =>
  snapshot.connectionTransport === "usb" && wiredMode === "aoa" && !snapshot.devices.some(device => isHarmony(device));
export const byTransport = (snapshot: Snapshot) => {
  const deviceTransport = snapshot.connectionTransport === "usb" ? "usb" : snapshot.settings.wirelessProtocol === "adb" ? "wifi" : null;
  return deviceTransport ? snapshot.devices.filter(device => device.transport === deviceTransport || device.transport === "unknown") : [];
};
export const hasActiveSession = (snapshot: Snapshot | null) => !!snapshot && (!!snapshot.sessionSerial || !!snapshot.sessionReceiverBinding || snapshot.sessionConnectionTransport !== null);
// `wirelessProtocol` is the effective one: a tablet target plays over UDP whatever is saved.
export const canSetAudioQuality = (snapshot: Snapshot, quality: Snapshot["settings"]["audioQuality"], wirelessProtocol = snapshot.settings.wirelessProtocol) => {
  const settings = snapshot.settings;
  const adbQualityOnly = settings.audioBackend === "legacy" && (snapshot.sessionTransport === "aoa" ? false : snapshot.sessionConnectionTransport !== null
    ? snapshot.sessionConnectionTransport === "usb" && settings.wiredMode !== "aoa"
      || snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "adb"
      : snapshot.connectionTransport === "usb" ? settings.wiredMode !== "aoa" : wirelessProtocol === "adb");
  return snapshot.canChangeModes && (!adbQualityOnly || quality === "standard");
};
export const connectionLabel = (transport: Snapshot["connectionTransport"] | Snapshot["sessionConnectionTransport"], protocol: Snapshot["sessionWirelessProtocol"] | null) =>
  transport === "usb" ? strings.connection.usb : transport === "usbDirect" ? strings.connection.usbDirect
    : transport === "wireless" ? protocol === "udp" ? strings.connection.udp : strings.connection.adb : strings.connection.unknown;
export const intendedConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.connectionTransport,
  snapshot.connectionTransport === "wireless" ? snapshot.settings.wirelessProtocol : null);
export const actualConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.sessionConnectionTransport, snapshot.sessionWirelessProtocol);
// Phone model per binding reference, from receiver_binding_profiles; bindings saved before models were recorded have none.
const bindingModels = new Map<string, string>();
export const rememberBindingModels = (profiles: Record<string, { model?: string | null }>) => {
  bindingModels.clear();
  for (const [reference, profile] of Object.entries(profiles)) if (profile.model) bindingModels.set(reference, profile.model);
};
// Choice-list labels: the phone model (repeats get " 2", " 3"), or "Saved phone N" when no model was recorded.
export const bindingChoiceLabels = (refs: string[]) => {
  const seen = new Map<string, number>();
  return new Map(refs.map((reference, index) => {
    const model = bindingModels.get(reference);
    if (!model) return [reference, strings.network.savedPhoneNumber(index + 1)];
    const count = (seen.get(model) ?? 0) + 1;
    seen.set(model, count);
    return [reference, count === 1 ? model : `${model} ${count}`];
  }));
};
// A phone request belongs to the saved phone the listener waits for; a lone saved phone owns every request.
export const offerBindingFor = (offers: { automaticBinding?: string | null }, references: string[]) =>
  offers.automaticBinding ? references.includes(offers.automaticBinding) ? offers.automaticBinding : null
    : references.length === 1 ? references[0] : null;
export const bindingWirelessProtocol = (saved: Snapshot["settings"]["wirelessProtocol"], profiles: Record<string, { platform?: string }>, reference: string | null | undefined) =>
  reference && profiles[reference]?.platform === "harmony" ? "udp" : saved;
export const harmonyBindingReferences = (profiles: Record<string, { platform?: string }>) =>
  Object.keys(profiles).filter(reference => profiles[reference].platform === "harmony");
export const bindingLabel = (reference: string | null | undefined) => (reference && bindingModels.get(reference)) || strings.connection.pairedReceiver;
export const bindingReferenceLabel = (reference: string | null | undefined) => !reference ? strings.connection.pairedReceiver
  : bindingModels.has(reference) ? `${bindingModels.get(reference)} · ${reference.slice(0, 8)}` : strings.connection.pairedReceiverWithReference(reference);
export const resetPhoneStateLabel = (state: string) => ({ empty: strings.network.resetPhoneEmpty, pending: strings.network.resetPhonePending, saved: strings.network.resetPhoneSaved, failed: strings.network.resetPhoneFailed, unknown: strings.network.resetPhoneUnknown } as Record<string, string>)[state] ?? state;
export const resetPcStateLabel = (state: string) => ({ missing: strings.network.resetPcMissing, saved: strings.network.resetPcSaved, unknown: strings.network.resetPcUnknown } as Record<string, string>)[state] ?? state;
export const resetOutcomeLabel = (outcome: string) => ({ reset: strings.network.resetOutcomeReset, alreadyEmpty: strings.network.resetOutcomeAlreadyEmpty, phoneStateUnknown: strings.network.resetOutcomePhoneUnknown, phoneRetained: strings.network.resetOutcomePhoneRetained, phoneClearedPcUnknown: strings.network.resetOutcomePhoneClearedPcUnknown, phoneClearedPcRetained: strings.network.resetOutcomePhoneClearedPcRetained, phoneClearedPcChanged: strings.network.resetOutcomePhoneClearedPcChanged, pcClearedPhoneEmpty: strings.network.resetOutcomePcClearedPhoneEmpty, pcClearedPhoneUnknown: strings.network.resetOutcomePcClearedPhoneUnknown, pcClearedPhoneChanged: strings.network.resetOutcomePcClearedPhoneChanged, targetChanged: strings.network.resetOutcomeTargetChanged } as Record<string, string>)[outcome] ?? strings.network.resetOutcomeReview;
export const commitmentSummary = (value: string | null) => strings.network.commitmentSummary(value);
export const playbackModeLabel = (mode: Snapshot["settings"]["playbackMode"]) => mode === "Balanced" ? strings.app.balanced : strings.app.lowLatency;
export const backgroundTrustKindLabel = (kind: BackgroundTrustStatus["kind"]) => ({
  none: strings.network.backgroundTrustNone, once: strings.network.backgroundTrustOnce,
  eightHours: strings.network.backgroundTrustEightHours, untilRevoked: strings.network.backgroundTrustUntilRevoked,
} as const)[kind];
export const backgroundTrustPersistenceLabel = (persistence: BackgroundTrustStatus["persistence"]) => ({
  none: strings.network.backgroundTrustPersistenceNone, process: strings.network.backgroundTrustPersistenceProcess,
  binding: strings.network.backgroundTrustPersistenceBinding,
} as const)[persistence];
export const backgroundTrustRemaining = (remainingMs: number | null) => remainingMs === null
  ? strings.network.backgroundTrustIndefinite
  : strings.network.backgroundTrustReportedRemaining(Math.ceil(remainingMs / 1000));
export const readableStatus = (snapshot: Snapshot, cleanupPending: boolean) => {
  if (cleanupPending) return strings.app.disconnectedPending;
  if (snapshot.wirelessConnectionOffers.active) return strings.app.waitingPhone;
  if (snapshot.state === "reconnecting" && snapshot.networkRecoverySecondsRemaining != null)
    return strings.app.reconnecting(snapshot.networkRecoverySecondsRemaining);
  if (snapshot.state === "streaming" && snapshot.output === "computer") return strings.app.connectedComputer;
  if (snapshot.state === "streaming" && snapshot.playbackDiagnostics?.playbackAdvancing !== true)
    return strings.app.receivingAudio;
  return ({
    noDevice: snapshot.devices.length ? strings.app.waitingAuthorization : snapshot.connectionTransport === "wireless" ? strings.app.choosePairedPhone : strings.app.noPhone,
    unauthorized: strings.app.waitingAuthorization, ready: strings.app.ready, connecting: strings.app.connecting, connected: strings.app.connectedComputer,
    streaming: strings.app.playingOnPhone, phoneReady: strings.app.phoneReady, paused: strings.app.paused,
    reconnecting: strings.app.reconnectingShort, managementOnly: strings.app.managementOnly, error: strings.app.error,
  } satisfies Record<Snapshot["state"], string>)[snapshot.state];
};
export const CAP_PLAYBACK_PROGRESS = 1 << 3;
export const CAP_AOA_SESSION_RESTART = 1 << 9;
export const needsReceiverUpdate = (message: string) => message.includes("RECEIVER_UPDATE_REQUIRED:");
export const windowsRateMismatchHz = (message: string) => {
  const rate = message.match(/FORMAT_RATE_MISMATCH:\s*actualHz=(\d+)/i)?.[1];
  return rate ? Number(rate) : null;
};
// The tablet prefix may follow the PC's own context (a stopped session's detail).
export const harmonyMessage = (message: string) => message.includes("HARMONY_START_REJECTED:") ? strings.app.harmonyStartRejected
  : message.includes("HARMONY_UNSUPPORTED:") ? strings.app.harmonyUnsupported : null;
// Headline for a failed operation; the technical message stays in the details.
export const noticeSummary = (message: string) => needsReceiverUpdate(message) ? strings.app.receiverUpdateRequired
  : harmonyMessage(message) ?? (message.includes("FORMAT_RATE_MISMATCH:") ? strings.app.hiresRateMismatch(windowsRateMismatchHz(message)) : strings.app.operationNotCompleted);
// Details without the internal tablet prefix.
export const errorDetail = (message: string) => message.replace(/HARMONY_(START_REJECTED|UNSUPPORTED):\s*/, "");
export const receiverErrorMessage = (message: string) => needsReceiverUpdate(message) ? strings.app.receiverUpdateRequired : harmonyMessage(message) ?? message;
export const isIpv4Endpoint = (value: string) => {
  const match = /^(\d{1,3}(?:\.\d{1,3}){3}):(\d{1,5})$/.exec(value.trim());
  return !!match && match[1].split(".").every(part => Number(part) <= 255) && Number(match[2]) > 0 && Number(match[2]) <= 65535;
};
export const ipv4Number = (address: string) => {
  const parts = address.split(".");
  if (parts.length !== 4 || parts.some(part => !/^\d{1,3}$/.test(part) || Number(part) > 255)) return null;
  return parts.reduce((value, part) => ((value << 8) | Number(part)) >>> 0, 0);
};
export const sameIpv4Subnet = (first: string, firstPrefix: number, second: string, secondPrefix: number) => {
  const a = ipv4Number(first); const b = ipv4Number(second);
  if (a === null || b === null || firstPrefix < 0 || firstPrefix > 32 || secondPrefix < 0 || secondPrefix > 32) return false;
  const prefix = Math.max(firstPrefix, secondPrefix);
  const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
  return (a & mask) >>> 0 === (b & mask) >>> 0;
};
export const bootstrapNetworkPairs = (locals: PrivateIpv4Interface[], receivers: ReceiverIpv4Interface[]): NetworkInterfacePair[] =>
  locals.flatMap(local => receivers.filter(receiver => sameIpv4Subnet(local.address, local.prefixLength, receiver.address, receiver.prefixLength))
    .map(receiver => ({ local, receiver })));
export const endpointFitsInterface = (endpoint: string, local: PrivateIpv4Interface | undefined) => {
  const address = endpoint.match(/^(\d{1,3}(?:\.\d{1,3}){3}):\d{1,5}$/)?.[1];
  return !!local && isIpv4Endpoint(endpoint) && !!address && sameIpv4Subnet(local.address, local.prefixLength, address, local.prefixLength);
};
