import { invoke } from "@tauri-apps/api/core";
import { strings } from "./strings";
import type { Snapshot, PrivateIpv4Interface, ReceiverIpv4Interface, NetworkInterfacePair, BackgroundTrustStatus } from "./appTypes";

export const isDesktop = "__TAURI_INTERNALS__" in window;
export const invokeApp = <T,>(command: string, args?: Record<string, unknown>) => {
  if (!isDesktop) return Promise.reject(new Error(strings.app.desktopOnly));
  return invoke<T>(command, args);
};
export const byTransport = (snapshot: Snapshot) => {
  const deviceTransport = snapshot.connectionTransport === "usb" ? "usb" : snapshot.settings.wirelessProtocol === "adb" ? "wifi" : null;
  return deviceTransport ? snapshot.devices.filter(device => device.transport === deviceTransport || device.transport === "unknown") : [];
};
export const hasActiveSession = (snapshot: Snapshot | null) => !!snapshot && (!!snapshot.sessionSerial || !!snapshot.sessionReceiverBinding || snapshot.sessionConnectionTransport !== null);
export const connectionLabel = (transport: Snapshot["connectionTransport"] | null, protocol: Snapshot["sessionWirelessProtocol"] | null) =>
  transport === "usb" ? strings.connection.usb : transport === "wireless" ? protocol === "udp" ? strings.connection.udp : strings.connection.adb : strings.connection.unknown;
export const intendedConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.connectionTransport,
  snapshot.connectionTransport === "wireless" ? snapshot.settings.wirelessProtocol : null);
export const actualConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.sessionConnectionTransport, snapshot.sessionWirelessProtocol);
export const bindingLabel = (_reference: string | null | undefined) => strings.connection.pairedReceiver;
export const bindingReferenceLabel = (reference: string | null | undefined) => reference ? strings.connection.pairedReceiverWithReference(reference) : strings.connection.pairedReceiver;
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
export const needsReceiverUpdate = (message: string) => message.includes("RECEIVER_UPDATE_REQUIRED:");
export const receiverErrorMessage = (message: string) => needsReceiverUpdate(message) ? strings.app.receiverUpdateRequired : message;
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
