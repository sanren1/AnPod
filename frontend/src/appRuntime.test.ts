import { beforeAll, describe, expect, it, vi } from "vitest";
import type { Snapshot } from "./appTypes";
import { strings } from "./strings";

// appRuntime reads `window` at import time; give it a non-Tauri window.
vi.stubGlobal("window", {});
let runtime: typeof import("./appRuntime");
beforeAll(async () => { runtime = await import("./appRuntime"); });

const snapshot = (overrides: Partial<Snapshot> = {}, settings: Partial<Snapshot["settings"]> = {}): Snapshot => ({
  state: "ready", message: "", deviceName: null, devices: [], selectedSerial: null, sessionSerial: null,
  sessionReceiverBinding: null, sessionTransport: null, sessionGeneration: null,
  sessionConnectionTransport: null, sessionWirelessProtocol: null, sessionAndroidVersion: null,
  connectionTransport: "usb", transport: "usb", presence: [], sessionCapabilities: null,
  wirelessConnectionOffers: { active: false, interfaceAddress: null, interfacePrefixLength: null, offers: [] },
  playbackDiagnostics: null, negotiatedAudioFormat: null, windowsMixFormat: null, androidOutputFormat: null,
  output: "phone", busy: false, canDisconnect: false, canChangeModes: true, canChangePlaybackMode: true,
  statusDetail: "", audioBitrateBps: null, version: "0.0.0", harmonyPairingCode: null,
  ...overrides,
  settings: {
    schemaVersion: 1, lastDeviceId: null, minimizeToTray: false, launchAtLogin: false, preferredTransport: "Usb",
    wiredMode: "adb", lastUsbDirectTargetId: null, lastUsbDirectBindingReference: null, wirelessProtocol: "udp",
    audioBackend: "receiver", lastWirelessEndpoint: null, lastReceiverEndpoint: null, audioQuality: "standard",
    playbackMode: "Balanced", appearanceTheme: "System",
    ...settings,
  },
});

describe("IPv4 helpers", () => {
  it("validates endpoints", () => {
    expect(runtime.isIpv4Endpoint(" 192.168.1.5:5555 ")).toBe(true);
    expect(runtime.isIpv4Endpoint("192.168.1.256:5555")).toBe(false);
    expect(runtime.isIpv4Endpoint("192.168.1.5:0")).toBe(false);
    expect(runtime.isIpv4Endpoint("192.168.1.5:65536")).toBe(false);
    expect(runtime.isIpv4Endpoint("192.168.1.5")).toBe(false);
  });

  it("converts addresses to unsigned numbers", () => {
    expect(runtime.ipv4Number("255.255.255.255")).toBe(0xffffffff);
    expect(runtime.ipv4Number("10.0.0.1")).toBe(0x0a000001);
    expect(runtime.ipv4Number("10.0.0")).toBeNull();
    expect(runtime.ipv4Number("10.0.0.x")).toBeNull();
  });

  it("compares subnets with the longer prefix", () => {
    expect(runtime.sameIpv4Subnet("192.168.1.2", 24, "192.168.1.200", 24)).toBe(true);
    expect(runtime.sameIpv4Subnet("192.168.1.2", 16, "192.168.2.2", 24)).toBe(false);
    expect(runtime.sameIpv4Subnet("10.1.2.3", 0, "172.16.0.1", 0)).toBe(true);
    expect(runtime.sameIpv4Subnet("10.1.2.3", 33, "10.1.2.3", 24)).toBe(false);
  });

  it("pairs only interfaces on the same subnet", () => {
    const wifi = { index: 1, name: "Wi-Fi", address: "192.168.1.2", prefixLength: 24, gateways: [] };
    const lan = { index: 2, name: "LAN", address: "10.0.0.2", prefixLength: 8, gateways: [] };
    const phone = { address: "192.168.1.9", prefixLength: 24, interfaceIndex: 3, interfaceName: "wlan0" };
    expect(runtime.bootstrapNetworkPairs([wifi, lan], [phone])).toEqual([{ local: wifi, receiver: phone }]);
    expect(runtime.endpointFitsInterface("192.168.1.9:40000", wifi)).toBe(true);
    expect(runtime.endpointFitsInterface("10.0.0.9:40000", wifi)).toBe(false);
    expect(runtime.endpointFitsInterface("192.168.1.9:40000", undefined)).toBe(false);
  });
});

describe("error message parsing", () => {
  it("detects receiver update requirement", () => {
    expect(runtime.needsReceiverUpdate("x RECEIVER_UPDATE_REQUIRED: 50")).toBe(true);
    expect(runtime.receiverErrorMessage("RECEIVER_UPDATE_REQUIRED:")).toBe(strings.app.receiverUpdateRequired);
    expect(runtime.receiverErrorMessage("other")).toBe("other");
  });

  it("extracts the Windows mix rate", () => {
    expect(runtime.windowsRateMismatchHz("FORMAT_RATE_MISMATCH: actualHz=44100")).toBe(44100);
    expect(runtime.windowsRateMismatchHz("FORMAT_RATE_MISMATCH")).toBeNull();
  });
});

describe("connection and quality", () => {
  it("labels intended and actual transports separately", () => {
    const value = snapshot({ connectionTransport: "wireless", sessionConnectionTransport: "usbDirect" }, { wirelessProtocol: "adb" });
    expect(runtime.intendedConnectionLabel(value)).toBe(strings.connection.adb);
    expect(runtime.actualConnectionLabel(value)).toBe(strings.connection.usbDirect);
    expect(runtime.actualConnectionLabel(snapshot())).toBe(strings.connection.unknown);
  });

  it("limits the legacy ADB backend to standard quality", () => {
    const legacyUsb = snapshot({}, { audioBackend: "legacy" });
    expect(runtime.canSetAudioQuality(legacyUsb, "standard")).toBe(true);
    expect(runtime.canSetAudioQuality(legacyUsb, "hires")).toBe(false);
    expect(runtime.canSetAudioQuality(snapshot({ sessionTransport: "aoa" }, { audioBackend: "legacy" }), "hires")).toBe(true);
    expect(runtime.canSetAudioQuality(snapshot({}, { audioBackend: "legacy", wiredMode: "aoa" }), "hires")).toBe(true);
    expect(runtime.canSetAudioQuality(snapshot({ canChangeModes: false }), "standard")).toBe(false);
  });

  it("detects an active session from any session field", () => {
    expect(runtime.hasActiveSession(null)).toBe(false);
    expect(runtime.hasActiveSession(snapshot())).toBe(false);
    expect(runtime.hasActiveSession(snapshot({ sessionReceiverBinding: "b" }))).toBe(true);
    expect(runtime.hasActiveSession(snapshot({ sessionConnectionTransport: "wireless" }))).toBe(true);
  });
});

describe("readableStatus", () => {
  it("does not report playing before playback advances", () => {
    expect(runtime.readableStatus(snapshot({ state: "streaming" }), false)).toBe(strings.app.receivingAudio);
    const advancing = { playbackAdvancing: true } as Snapshot["playbackDiagnostics"];
    expect(runtime.readableStatus(snapshot({ state: "streaming", playbackDiagnostics: advancing }), false)).toBe(strings.app.playingOnPhone);
    expect(runtime.readableStatus(snapshot({ state: "streaming", output: "computer" }), false)).toBe(strings.app.connectedComputer);
  });

  it("prioritizes cleanup, offers and recovery countdown", () => {
    expect(runtime.readableStatus(snapshot({ state: "streaming" }), true)).toBe(strings.app.disconnectedPending);
    const offers = { active: true, interfaceAddress: null, interfacePrefixLength: null, offers: [] };
    expect(runtime.readableStatus(snapshot({ wirelessConnectionOffers: offers }), false)).toBe(strings.app.waitingPhone);
    expect(runtime.readableStatus(snapshot({ state: "reconnecting", networkRecoverySecondsRemaining: 5 }), false)).toBe(strings.app.reconnecting(5));
    expect(runtime.readableStatus(snapshot({ state: "reconnecting" }), false)).toBe(strings.app.reconnectingShort);
  });

  it("distinguishes no device by transport", () => {
    expect(runtime.readableStatus(snapshot({ state: "noDevice" }), false)).toBe(strings.app.noPhone);
    expect(runtime.readableStatus(snapshot({ state: "noDevice", connectionTransport: "wireless" }), false)).toBe(strings.app.choosePairedPhone);
  });
});

describe("binding labels", () => {
  it("prefer the saved phone model and keep the reference for older bindings", () => {
    const withModel = "a".repeat(64);
    const older = "b".repeat(64);
    runtime.rememberBindingModels({ [withModel]: { model: "Pixel 8" }, [older]: {} });
    expect(runtime.bindingLabel(withModel)).toBe("Pixel 8");
    expect(runtime.bindingReferenceLabel(withModel)).toBe("Pixel 8 · aaaaaaaa");
    expect(runtime.bindingLabel(older)).toBe(strings.connection.pairedReceiver);
    expect(runtime.bindingReferenceLabel(older)).toBe(strings.connection.pairedReceiverWithReference(older));
    runtime.rememberBindingModels({});
    expect(runtime.bindingLabel(withModel)).toBe(strings.connection.pairedReceiver);
  });
});

describe("deviceOsLabel", () => {
  const base = { serial: "s", transport: "usb", state: "device", manufacturer: "", model: "", apiLevel: "", displayName: "d", statusText: "", connectable: true } as const;
  it("labels harmony and android devices", () => {
    expect(runtime.deviceOsLabel({ ...base, platform: "harmony", androidVersion: "", osVersion: "HarmonyOS 7" })).toBe("HarmonyOS 7");
    expect(runtime.deviceOsLabel({ ...base, platform: "android", androidVersion: "15", osVersion: "15" })).toBe("Android 15");
    expect(runtime.deviceOsLabel({ ...base, platform: "android", androidVersion: "", osVersion: "" })).toBe("");
  });
});

describe("HarmonyOS wired mode", () => {
  const base = { serial: "s", transport: "usb", state: "device", manufacturer: "", model: "", apiLevel: "", displayName: "d", statusText: "", connectable: true, androidVersion: "", osVersion: "" } as const;
  it("harmony device hides adb and usb direct and keeps wiredMode", () => {
    const harmony = { ...base, platform: "harmony" } as const;
    const android = { ...base, platform: "android" } as const;
    expect(runtime.wiredModeChoice("aoa", harmony)).toEqual({ value: "hdc", options: ["hdc"] });
    expect(runtime.wiredModeChoice("adb", harmony)).toEqual({ value: "hdc", options: ["hdc"] });
    expect(runtime.wiredModeChoice("aoa", android)).toEqual({ value: "aoa", options: ["adb", "aoa"] });
    expect(runtime.wiredModeChoice("adb", undefined)).toEqual({ value: "adb", options: ["adb", "aoa"] });
    // Routing for a tablet never takes the saved USB Direct choice, and the saved value is untouched.
    expect(runtime.effectiveWiredMode("aoa", harmony)).toBe("adb");
    expect(runtime.effectiveWiredMode("aoa", android)).toBe("aoa");
  });
  it("lets the picker list a tablet even when USB Direct is saved", () => {
    const harmony = { ...base, serial: "HW1", platform: "harmony" } as const;
    const android = { ...base, platform: "android" } as const;
    // Android only: USB Direct mode keeps opening the USB Direct dialog.
    expect(runtime.pickerOpensUsbDirect(snapshot({ devices: [android] }, { wiredMode: "aoa" }), "aoa")).toBe(true);
    expect(runtime.pickerOpensUsbDirect(snapshot({ devices: [android] }, { wiredMode: "adb" }), "adb")).toBe(false);
    // Android selected by default but a tablet is attached: the device list opens instead.
    expect(runtime.pickerOpensUsbDirect(snapshot({ devices: [android, harmony] }, { wiredMode: "aoa" }), "aoa")).toBe(false);
  });
});

it("selects UDP for a paired Harmony target while preserving the saved Android preference", () => {
  const saved = snapshot({}, { wirelessProtocol: "adb" }).settings;
  const profiles = { tablet: { platform: "harmony" }, legacy: {} };
  expect(runtime.bindingWirelessProtocol(saved.wirelessProtocol, profiles, "tablet")).toBe("udp");
  expect(runtime.bindingWirelessProtocol(saved.wirelessProtocol, profiles, "legacy")).toBe("adb");
  expect(runtime.bindingWirelessProtocol(saved.wirelessProtocol, profiles, null)).toBe("adb");
  expect(saved.wirelessProtocol).toBe("adb");
});

it("offers the PC-side forget entry only for tablet bindings", () => {
  const profiles = { phone: { platform: "android" }, legacy: {}, tablet: { platform: "harmony" } };
  expect(runtime.harmonyBindingReferences(profiles)).toEqual(["tablet"]);
  expect(runtime.harmonyBindingReferences({})).toEqual([]);
});

it("explains a tablet START rejection instead of the generic failure", () => {
  expect(runtime.noticeSummary("HARMONY_START_REJECTED:平板拒绝了此档位，详情见平板。")).toBe(strings.app.harmonyStartRejected);
  expect(runtime.noticeSummary("RECEIVER_UPDATE_REQUIRED: x")).toBe(strings.app.receiverUpdateRequired);
  expect(runtime.noticeSummary("FORMAT_RATE_MISMATCH: actualHz=48000")).toBe(strings.app.hiresRateMismatch(48000));
  expect(runtime.noticeSummary("other")).toBe(strings.app.operationNotCompleted);
});
