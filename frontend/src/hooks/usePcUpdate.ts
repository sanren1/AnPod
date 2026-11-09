import { useRef, useState } from "react";
import { invokeApp } from "../appRuntime";
import type { PcUpdateState } from "../components/SettingsDialog";

type UpdateCheck = { currentVersion: string; latestVersion: string; available: boolean; releaseUrl: string; assetName: string; assetSize: number };

/** PC self-update state; one lock keeps check, download, install and open from overlapping. */
export function usePcUpdate() {
  const [pcUpdate, setPcUpdate] = useState<PcUpdateState>({ phase: "idle" });
  const lock = useRef(false);
  const fail = (failedAction: PcUpdateState["failedAction"], error: unknown, extra: Partial<PcUpdateState> = {}) =>
    setPcUpdate(current => ({ ...current, ...extra, phase: "error", failedAction, errorMessage: String(error) }));

  const check = async () => {
    if (lock.current || pcUpdate.phase === "installing") return;
    lock.current = true;
    setPcUpdate(current => ({ phase: "checking", installResult: current.installResult }));
    try {
      const result = await invokeApp<UpdateCheck>("check_pc_update");
      setPcUpdate(current => ({ phase: "idle", ...result, prepared: false, installResult: current.installResult }));
    } catch (error) {
      setPcUpdate(current => ({ phase: "error", failedAction: "check", errorMessage: String(error), installResult: current.installResult }));
    } finally {
      lock.current = false;
    }
  };
  const download = async () => {
    if (lock.current || pcUpdate.phase === "installing" || !pcUpdate.available || pcUpdate.prepared) return;
    lock.current = true;
    setPcUpdate(current => ({ ...current, phase: "downloading", errorMessage: undefined }));
    try {
      await invokeApp<{ status: "prepared" }>("download_pc_update");
      setPcUpdate(current => ({ ...current, phase: "ready", prepared: true }));
    } catch (error) {
      fail("download", error, { prepared: false });
    } finally {
      lock.current = false;
    }
  };
  /** `blocked` is true while another operation or an audio session is running. */
  const install = async (blocked: boolean) => {
    if (lock.current || blocked || !pcUpdate.prepared) return;
    lock.current = true;
    setPcUpdate(current => ({ ...current, phase: "installing", errorMessage: undefined }));
    try { await invokeApp("install_pc_update"); }
    catch (error) { fail("install", error); }
    finally { lock.current = false; }
  };
  const open = async (command: "open_pc_update_download" | "open_pc_update_release", allowed: boolean) => {
    if (lock.current || pcUpdate.phase === "installing" || !allowed) return;
    lock.current = true;
    try { await invokeApp(command); }
    catch (error) { fail("open", error); }
    finally { lock.current = false; }
  };
  const loadStatus = async () => {
    try {
      const result = await invokeApp<{ installResult: PcUpdateState["installResult"] }>("get_pc_update_status");
      if (result.installResult) setPcUpdate(current => ({ ...current, installResult: result.installResult }));
    } catch (error) {
      fail("status", error);
    }
  };

  return {
    pcUpdate,
    checkPcUpdate: check,
    downloadPcUpdate: download,
    installPcUpdate: install,
    openPcUpdateDownload: () => open("open_pc_update_download", !!pcUpdate.prepared),
    openPcUpdateRelease: () => open("open_pc_update_release", !!pcUpdate.releaseUrl),
    loadPcUpdateStatus: loadStatus,
  };
}
