import { invoke } from "@tauri-apps/api/core";
import { zhCN } from "./strings.zh-CN";
import type { Snapshot, PrivateIpv4Interface, ReceiverIpv4Interface, NetworkInterfacePair, BackgroundTrustStatus } from "./appTypes";

export const isDesktop = "__TAURI_INTERNALS__" in window;
export const invokeApp = <T,>(command: string, args?: Record<string, unknown>) => {
  if (!isDesktop) return Promise.reject(new Error(zhCN.app.desktopOnly));
  return invoke<T>(command, args);
};
export const byTransport = (snapshot: Snapshot) => {
  const deviceTransport = snapshot.connectionTransport === "usb" ? "usb" : snapshot.settings.wirelessProtocol === "adb" ? "wifi" : null;
  return deviceTransport ? snapshot.devices.filter(device => device.transport === deviceTransport || device.transport === "unknown") : [];
};
export const hasActiveSession = (snapshot: Snapshot | null) => !!snapshot && (!!snapshot.sessionSerial || !!snapshot.sessionReceiverBinding || snapshot.sessionConnectionTransport !== null);
export const connectionLabel = (transport: Snapshot["connectionTransport"] | null, protocol: Snapshot["sessionWirelessProtocol"] | null) =>
  transport === "usb" ? zhCN.connection.usb : transport === "wireless" ? protocol === "udp" ? zhCN.connection.udp : zhCN.connection.adb : zhCN.connection.unknown;
export const intendedConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.connectionTransport,
  snapshot.connectionTransport === "wireless" ? snapshot.settings.wirelessProtocol : null);
export const actualConnectionLabel = (snapshot: Snapshot) => connectionLabel(snapshot.sessionConnectionTransport, snapshot.sessionWirelessProtocol);
export const bindingLabel = (_reference: string | null | undefined) => zhCN.connection.pairedReceiver;
export const bindingReferenceLabel = (reference: string | null | undefined) => reference ? zhCN.connection.pairedReceiverWithReference(reference) : zhCN.connection.pairedReceiver;
export const resetPhoneStateLabel = (state: string) => ({ empty: zhCN.network.resetPhoneEmpty, pending: zhCN.network.resetPhonePending, saved: zhCN.network.resetPhoneSaved, failed: zhCN.network.resetPhoneFailed, unknown: zhCN.network.resetPhoneUnknown } as Record<string, string>)[state] ?? state;
export const resetPcStateLabel = (state: string) => ({ missing: zhCN.network.resetPcMissing, saved: zhCN.network.resetPcSaved, unknown: zhCN.network.resetPcUnknown } as Record<string, string>)[state] ?? state;
export const resetOutcomeLabel = (outcome: string) => ({ reset: zhCN.network.resetOutcomeReset, alreadyEmpty: zhCN.network.resetOutcomeAlreadyEmpty, phoneStateUnknown: zhCN.network.resetOutcomePhoneUnknown, phoneRetained: zhCN.network.resetOutcomePhoneRetained, phoneClearedPcUnknown: zhCN.network.resetOutcomePhoneClearedPcUnknown, phoneClearedPcRetained: zhCN.network.resetOutcomePhoneClearedPcRetained, phoneClearedPcChanged: zhCN.network.resetOutcomePhoneClearedPcChanged, pcClearedPhoneEmpty: zhCN.network.resetOutcomePcClearedPhoneEmpty, pcClearedPhoneUnknown: zhCN.network.resetOutcomePcClearedPhoneUnknown, pcClearedPhoneChanged: zhCN.network.resetOutcomePcClearedPhoneChanged, targetChanged: zhCN.network.resetOutcomeTargetChanged } as Record<string, string>)[outcome] ?? zhCN.network.resetOutcomeReview;
export const commitmentSummary = (value: string | null) => zhCN.network.commitmentSummary(value);
export const playbackModeLabel = (mode: Snapshot["settings"]["playbackMode"]) => mode === "Balanced" ? zhCN.app.balanced : zhCN.app.lowLatency;
export const backgroundTrustKindLabel = (kind: BackgroundTrustStatus["kind"]) => ({
  none: zhCN.network.backgroundTrustNone, once: zhCN.network.backgroundTrustOnce,
  eightHours: zhCN.network.backgroundTrustEightHours, untilRevoked: zhCN.network.backgroundTrustUntilRevoked,
} as const)[kind];
export const backgroundTrustPersistenceLabel = (persistence: BackgroundTrustStatus["persistence"]) => ({
  none: zhCN.network.backgroundTrustPersistenceNone, process: zhCN.network.backgroundTrustPersistenceProcess,
  binding: zhCN.network.backgroundTrustPersistenceBinding,
} as const)[persistence];
export const backgroundTrustRemaining = (remainingMs: number | null) => remainingMs === null
  ? zhCN.network.backgroundTrustIndefinite
  : zhCN.network.backgroundTrustReportedRemaining(Math.ceil(remainingMs / 1000));
export const readableStatus = (snapshot: Snapshot, cleanupPending: boolean) => {
  if (cleanupPending) return zhCN.app.disconnectedPending;
  if (snapshot.wirelessConnectionOffers.active) return zhCN.app.waitingPhone;
  if (snapshot.state === "reconnecting" && snapshot.networkRecoverySecondsRemaining != null)
    return zhCN.app.reconnecting(snapshot.networkRecoverySecondsRemaining);
  if (snapshot.state === "streaming" && snapshot.output === "computer") return zhCN.app.connectedComputer;
  if (snapshot.state === "streaming" && snapshot.playbackDiagnostics?.playbackAdvancing !== true)
    return zhCN.app.receivingAudio;
  return ({
    noDevice: snapshot.devices.length ? zhCN.app.waitingAuthorization : snapshot.connectionTransport === "wireless" ? zhCN.app.choosePairedPhone : zhCN.app.noPhone,
    unauthorized: zhCN.app.waitingAuthorization, ready: zhCN.app.ready, connecting: zhCN.app.connecting, connected: zhCN.app.connectedComputer,
    streaming: zhCN.app.playingOnPhone, phoneReady: zhCN.app.phoneReady, paused: zhCN.app.paused,
    reconnecting: zhCN.app.reconnectingShort, managementOnly: zhCN.app.managementOnly, error: zhCN.app.error,
  } satisfies Record<Snapshot["state"], string>)[snapshot.state];
};
export const CAP_PLAYBACK_PROGRESS = 1 << 3;
export const needsReceiverUpdate = (message: string) => message.includes("RECEIVER_UPDATE_REQUIRED:");
export const receiverErrorMessage = (message: string) => needsReceiverUpdate(message) ? zhCN.app.receiverUpdateRequired : message;
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
