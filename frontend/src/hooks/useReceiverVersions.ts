import { useEffect, useRef, useState } from "react";
import { invokeApp, isDesktop } from "../appRuntime";
import type { ReceiverPackageCandidate, Snapshot } from "../appTypes";

/**
 * Installed Receiver versionCode per USB serial (null = not installed), read once when the phone
 * appears, and the first connectable USB phone older than the bundled package.
 */
export function useReceiverVersions(snapshot: Snapshot | null) {
  const [installedReceiverVersions, setInstalledReceiverVersions] = useState<Record<string, number | null>>({});
  const [bundledReceiverVersion, setBundledReceiverVersion] = useState<number | null>(null);
  const checked = useRef(new Set<string>());
  const bundledRequested = useRef(false);

  useEffect(() => {
    if (!isDesktop || !snapshot) return;
    const usbSerials = snapshot.devices.filter(device => device.transport === "usb" && device.connectable).map(device => device.serial);
    // A phone that leaves and comes back is checked again; no polling in between.
    for (const serial of checked.current) if (!usbSerials.includes(serial)) checked.current.delete(serial);
    const unchecked = usbSerials.filter(serial => !checked.current.has(serial));
    if (unchecked.length === 0) return;
    if (!bundledRequested.current) {
      bundledRequested.current = true;
      void invokeApp<ReceiverPackageCandidate | null>("receiver_update_candidate").then(candidate => setBundledReceiverVersion(candidate?.versionCode ?? null)).catch(() => {});
    }
    for (const serial of unchecked) {
      checked.current.add(serial);
      void invokeApp<number | null>("receiver_installed_version", { serial })
        .then(version => setInstalledReceiverVersions(current => ({ ...current, [serial]: version })))
        .catch(() => {});
    }
  }, [snapshot]);

  const outdatedReceiverSerial = bundledReceiverVersion === null ? undefined : snapshot?.devices.find(device => {
    const installed = installedReceiverVersions[device.serial];
    return device.transport === "usb" && device.connectable && typeof installed === "number" && installed < bundledReceiverVersion;
  })?.serial;
  const rememberInstalledVersion = (serial: string, version: number | null) =>
    setInstalledReceiverVersions(current => ({ ...current, [serial]: version }));

  return { outdatedReceiverSerial, rememberInstalledVersion };
}
