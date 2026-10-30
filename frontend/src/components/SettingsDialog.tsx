import { useEffect, useRef, useState } from "react";
import { strings } from "../strings";
import type { Settings, Snapshot } from "../appTypes";
import { Activity, CircleHelp, Smartphone } from "lucide-react";
import { ErrorDetails, Modal } from "./Modal";
import { SettingToggle } from "./SettingToggle";
import { canSetAudioQuality, hasActiveSession } from "../appRuntime";

type SettingsTab = "general" | "audio" | "connection" | "about";
export type PcUpdateState = { phase: "idle" | "checking" | "downloading" | "installing" | "ready" | "error"; currentVersion?: string; latestVersion?: string; available?: boolean; prepared?: boolean; releaseUrl?: string; assetName?: string; assetSize?: number; failedAction?: "check" | "download" | "install" | "open" | "status"; errorMessage?: string; installResult?: { status: "installed" | "failed" | "rolledBack" | "rollbackFailed"; message: string } };

export function SettingsDialog({ snapshot, largeText, onLargeText, udpBackendFixed, aoaSession, busy, updateState, onCheckUpdate, onDownloadUpdate, onInstallUpdate, onOpenUpdateDownload, onOpenUpdateRelease, onLoadUpdateStatus, onSetOption, onOpenReceiverManager, onOpenDiagnostics, onOpenHelp, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; udpBackendFixed: boolean; aoaSession: boolean; busy: boolean; updateState: PcUpdateState; onCheckUpdate: () => void; onDownloadUpdate: () => void; onInstallUpdate: () => void; onOpenUpdateDownload: () => void; onOpenUpdateRelease: () => void; onLoadUpdateStatus: () => void; onSetOption: (kind: "wirelessProtocol" | "audioBackend", value: string) => Promise<void>; onOpenReceiverManager: () => void; onOpenDiagnostics: () => void; onOpenHelp: () => void; onSave: (key: keyof Settings, value: unknown) => Promise<void>; onClose: () => void }) {
  const [savingKeys, setSavingKeys] = useState<Set<keyof Settings>>(() => new Set());
  const [saveError, setSaveError] = useState(false);
  const [tab, setTab] = useState<SettingsTab>("general");
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const updateStatusLoaded = useRef(false);
  useEffect(() => {
    if (tab !== "about" || updateStatusLoaded.current) return;
    updateStatusLoaded.current = true;
    onLoadUpdateStatus();
  }, [tab, onLoadUpdateStatus]);
  const settings = snapshot.settings;
  const tabs: { id: SettingsTab; label: string }[] = [
    { id: "general", label: strings.app.settingsGeneral },
    { id: "audio", label: strings.app.settingsAudio },
    { id: "connection", label: strings.app.settingsConnection },
    { id: "about", label: strings.app.settingsAbout },
  ];
  const selectTab = (index: number) => {
    const next = (index + tabs.length) % tabs.length;
    setTab(tabs[next].id);
    tabRefs.current[next]?.focus();
  };
  const update = async (key: keyof Settings, value: boolean | string, save: () => Promise<void> = () => onSave(key, value)) => {
    setSavingKeys(current => new Set(current).add(key));
    setSaveError(false);
    try { await save(); }
    catch { setSaveError(true); }
    finally { setSavingKeys(current => { const next = new Set(current); next.delete(key); return next; }); }
  };
  const negotiated = snapshot.negotiatedAudioFormat;
  const mix = snapshot.windowsMixFormat;
  const android = snapshot.androidOutputFormat;
  const actualFormat = negotiated
    ? [
      `${negotiated.sampleRateHz / 1000} kHz`,
      strings.app.audioSampleFormats[negotiated.sampleFormat],
      `${negotiated.channels} ch`,
      ...(negotiated.sampleFormat === "float32le" ? [] : [`${negotiated.validBitsPerSample} ${strings.app.validBits}`]),
    ].join(" · ")
    : strings.app.audioFormatNotConfirmed;
  const mixFormat = mix
    ? [
      mix.sampleRate === null ? strings.app.audioFormatUnknown : `${mix.sampleRate / 1000} kHz`,
      mix.sampleFormat === null ? strings.app.audioFormatUnknown : mix.sampleFormat === "float"
        ? `${strings.app.mixSampleFormats.float}${mix.containerBits === null ? "" : mix.containerBits}`
        : strings.app.mixSampleFormats.integer,
      mix.channels === null ? strings.app.audioFormatUnknown : `${mix.channels} ch`,
      ...(mix.sampleFormat === "integer" ? [
        mix.containerBits === null ? strings.app.audioFormatUnknown : `${mix.containerBits} ${strings.app.containerBits}`,
        mix.validBits === null ? strings.app.audioFormatUnknown : `${mix.validBits} ${strings.app.validBits}`,
      ] : []),
    ].join(" · ")
    : strings.app.audioFormatUnknown;
  const outputFormat = android
    ? `${android.sampleRate === null ? strings.app.audioFormatUnknown : `${android.sampleRate / 1000} kHz`} · ${android.encoding === null ? strings.app.audioFormatUnknown : strings.app.androidSampleFormats[android.encoding]} · ${android.channels === null ? strings.app.audioFormatUnknown : `${android.channels} ch`}`
    : strings.app.audioFormatUnknown;
  const activeSession = busy || snapshot.busy || hasActiveSession(snapshot);
  const checking = updateState.phase === "checking";
  const downloading = updateState.phase === "downloading";
  const updateBusy = checking || downloading || updateState.phase === "installing";
  const updateMessage = updateState.phase === "checking" ? strings.app.pcUpdateChecking
    : updateState.phase === "downloading" ? strings.app.pcUpdateDownloading
      : updateState.phase === "installing" ? strings.app.pcUpdateInstalling
      : updateState.phase === "ready" ? strings.app.pcUpdateReady
        : updateState.phase === "error" ? strings.app.pcUpdateFailed(updateState.failedAction ?? "check")
          : updateState.latestVersion ? updateState.available ? strings.app.pcUpdateAvailable : strings.app.pcUpdateCurrent
            : "";
  return <Modal title={strings.app.settingsTitle} onClose={onClose} className="settings-dialog">
    <div className="settings-tabs" role="tablist" aria-label={strings.app.settingsTitle}>
      {tabs.map(({ id, label }, index) => <button key={id} ref={node => { tabRefs.current[index] = node; }} type="button" role="tab" id={`settings-tab-${id}`} aria-controls={`settings-panel-${id}`} aria-selected={tab === id} tabIndex={tab === id ? 0 : -1} onClick={() => setTab(id)} onKeyDown={event => {
        if (event.key === "ArrowRight") { event.preventDefault(); selectTab(index + 1); }
        else if (event.key === "ArrowLeft") { event.preventDefault(); selectTab(index - 1); }
        else if (event.key === "Home") { event.preventDefault(); selectTab(0); }
        else if (event.key === "End") { event.preventDefault(); selectTab(tabs.length - 1); }
      }}>{label}</button>)}
    </div>
    <div className="settings-panel">
      {(savingKeys.size > 0 || saveError) && <div className="settings-save-feedback">
        {savingKeys.size > 0 && <p className="dialog-footnote" role="status" aria-live="polite">{strings.app.settingsSavePending}</p>}
        {saveError && <p className="status-description actionable-error" role="alert">{strings.app.settingsSaveFailed}</p>}
      </div>}
      <div className="settings-tab-panel" role="tabpanel" id="settings-panel-general" aria-labelledby="settings-tab-general" tabIndex={0} hidden={tab !== "general"}><section className="settings-group settings-appearance" aria-label={strings.app.settingsGeneral}>
        <label className="select-setting"><span>{strings.app.theme}</span><select disabled={snapshot.busy || savingKeys.has("appearanceTheme")} value={settings.appearanceTheme} onChange={event => void update("appearanceTheme", event.target.value)}><option value="System">{strings.app.followSystem}</option><option value="Light">{strings.app.light}</option><option value="Dark">{strings.app.dark}</option></select></label>
        <SettingToggle title={strings.app.enlargeText} checked={largeText} onChange={onLargeText} />
        <SettingToggle disabled={snapshot.busy || savingKeys.has("minimizeToTray")} title={strings.app.minimizeToTray} description={strings.app.minimizeToTrayDescription} checked={settings.minimizeToTray} onChange={value => void update("minimizeToTray", value)} />
        <SettingToggle disabled={snapshot.busy || savingKeys.has("launchAtLogin")} title={strings.app.launchAtLogin} checked={settings.launchAtLogin} onChange={value => void update("launchAtLogin", value)} />
      </section></div>
      <div className="settings-tab-panel" role="tabpanel" id="settings-panel-audio" aria-labelledby="settings-tab-audio" tabIndex={0} hidden={tab !== "audio"}><section className="settings-group settings-audio" aria-label={strings.app.settingsAudio}>
        <fieldset className="audio-quality-settings" aria-label={strings.app.transportQuality} aria-describedby="audio-quality-help">
          <div className="audio-quality-options">
            {(["standard", "hifi", "hires"] as const).map(quality => {
              const fullLabel = strings.app.audioQualityLabels[quality];
              const requestedSpec = fullLabel.split(" · ").slice(1).join(" · ").replace(/\s*(?:（实验）|\(experimental\))$/i, "");
              return <label className="audio-quality-card" key={quality}>
                <input type="radio" name="audioQuality" value={quality} aria-label={fullLabel} checked={settings.audioQuality === quality} disabled={busy || savingKeys.has("audioQuality") || !canSetAudioQuality(snapshot, quality)} onChange={() => void update("audioQuality", quality)} />
                <span><strong>{strings.app.audioQualityShortLabels[quality]}</strong><small>{requestedSpec}</small>{quality !== "standard" && <small>{strings.app.audioExperimental}</small>}</span>
              </label>;
            })}
          </div>
          <p id="audio-quality-help" className="dialog-footnote">{strings.app.audioQualityInDevelopment}</p>
          <details className="audio-format-disclosure">
            <summary>{strings.app.audioFormatDetails}</summary>
            <dl className="audio-quality-status">
              <div><dt>{strings.app.actualTransportFormat}</dt><dd>{actualFormat}</dd></div>
              <div><dt>{strings.app.windowsMixFormat}</dt><dd>{mixFormat}</dd></div>
              <div><dt>{strings.app.androidOutputFormat}</dt><dd>{outputFormat}</dd></div>
            </dl>
            <p className="dialog-footnote">{strings.app.audioOutputPrecisionNotice}</p>
          </details>
        </fieldset>
      </section></div>
      <div className="settings-tab-panel" role="tabpanel" id="settings-panel-connection" aria-labelledby="settings-tab-connection" tabIndex={0} hidden={tab !== "connection"}>
        <section className="settings-group settings-connection">
          <label className="select-setting"><span>{strings.app.wirelessProtocol}<small>{strings.app.wirelessProtocolHelp}</small></span><select aria-label={strings.app.wirelessProtocol} disabled={busy || savingKeys.has("wirelessProtocol")} value={settings.wirelessProtocol} onChange={event => { const value = event.target.value; void update("wirelessProtocol", value, () => onSetOption("wirelessProtocol", value)); }}><option value="udp">{strings.app.udpRecommended}</option><option value="adb">{strings.app.wirelessAdbCompatible}</option></select></label>
          <label className="select-setting"><span>{strings.app.receiverBackend}<small>{strings.app.receiverBackendHelp}</small></span><select aria-label={strings.app.receiverBackend} disabled={busy || udpBackendFixed || savingKeys.has("audioBackend")} value={settings.audioBackend} onChange={event => { const value = event.target.value; void update("audioBackend", value, () => onSetOption("audioBackend", value)); }}><option value="legacy">{strings.app.legacyCompatible}</option><option value="receiver">{strings.app.receiverModeRecommended}</option></select></label>
          {settings.wirelessProtocol === "adb" && <p className="dialog-footnote">{strings.app.wirelessAdbHelp}</p>}
          {aoaSession ? <p className="dialog-footnote">{strings.app.aoaReceiverBackendFixed}</p> : udpBackendFixed && <p className="dialog-footnote">{strings.app.udpUsesReceiver}</p>}
        </section>
        <section className="settings-group settings-connection-shortcut" aria-labelledby="settings-phone-title">
          <h3 id="settings-phone-title">{strings.app.settingsPhoneApp}</h3>
          <div className="settings-about-actions"><button type="button" disabled={busy} onClick={onOpenReceiverManager}><Smartphone size={15} aria-hidden="true" />{strings.app.phoneApplication}</button></div>
        </section>
      </div>
      <div className="settings-tab-panel" role="tabpanel" id="settings-panel-about" aria-labelledby="settings-tab-about" tabIndex={0} hidden={tab !== "about"}>
        <section className="settings-group settings-about">
          <p className="settings-version">{strings.app.pcVersion}: <strong>{snapshot.version}</strong></p>
          <div className="settings-update-actions">
            <button type="button" disabled={updateBusy} onClick={onCheckUpdate}>{checking ? strings.app.pcUpdateChecking : strings.app.pcUpdateCheck}</button>
            <button type="button" disabled={updateBusy || !updateState.available || updateState.prepared} onClick={onDownloadUpdate}>{downloading ? strings.app.pcUpdateDownloading : strings.app.pcUpdateDownload}</button>
            <button type="button" className="primary-button" disabled={!updateState.prepared || updateBusy || activeSession} onClick={onInstallUpdate}>{strings.app.pcUpdateInstall}</button>
          </div>
          {updateMessage && !(updateState.phase === "error" && updateState.errorMessage) && <p className={`dialog-footnote ${updateState.phase === "error" ? "actionable-error" : ""}`} role={updateState.phase === "error" ? "alert" : "status"} aria-live="polite">{updateMessage}</p>}
          {updateState.errorMessage && updateState.phase === "error" && <ErrorDetails className="settings-update-error" summary={strings.app.pcUpdateFailed(updateState.failedAction ?? "check")} details={updateState.errorMessage} />}
          {updateState.installResult && <p className={`dialog-footnote ${updateState.installResult.status === "installed" ? "" : "actionable-error"}`} role="status">{updateState.installResult.message}</p>}
          {updateState.assetName && <p className="dialog-footnote settings-update-asset">{updateState.assetName}{updateState.assetSize ? ` · ${(updateState.assetSize / 1024 / 1024).toFixed(1)} MB` : ""}</p>}
          <div className="settings-about-actions settings-update-links">
            <button type="button" disabled={!updateState.releaseUrl || updateBusy} onClick={onOpenUpdateRelease}>{strings.app.pcUpdateRelease}</button>
            <button type="button" disabled={!updateState.prepared || updateBusy} onClick={onOpenUpdateDownload}>{strings.app.pcUpdateOpenDownloads}</button>
          </div>
          {activeSession && <p className="dialog-footnote">{strings.app.pcUpdateDisconnectFirst}</p>}
        </section>
        <section className="settings-group settings-about" aria-labelledby="settings-support-title">
          <h3 id="settings-support-title">{strings.app.settingsSupport}</h3>
          <div className="settings-about-actions">
            <button type="button" onClick={onOpenDiagnostics}><Activity size={15} aria-hidden="true" />{strings.app.diagnostics}</button>
            <button type="button" onClick={onOpenHelp}><CircleHelp size={15} aria-hidden="true" />{strings.app.help}</button>
          </div>
          <p className="dialog-footnote">{strings.app.settingsPrivacy}</p>
        </section>
      </div>
    </div>
  </Modal>;
}
