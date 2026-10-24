import { useState } from "react";
import { strings } from "../strings";
import type { Settings, Snapshot } from "../appTypes";
import { Activity, CircleHelp, Smartphone } from "lucide-react";
import { Modal } from "./Modal";
import { SettingToggle } from "./SettingToggle";

export function SettingsDialog({ snapshot, largeText, onLargeText, udpBackendFixed, aoaSession, busy, onSetOption, onOpenReceiverManager, onOpenDiagnostics, onOpenHelp, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; udpBackendFixed: boolean; aoaSession: boolean; busy: boolean; onSetOption: (kind: "wirelessProtocol" | "audioBackend", value: string) => Promise<void>; onOpenReceiverManager: () => void; onOpenDiagnostics: () => void; onOpenHelp: () => void; onSave: (key: string, value: unknown) => Promise<void>; onClose: () => void }) {
  const [savingKeys, setSavingKeys] = useState<Set<keyof Settings>>(() => new Set());
  const [saveError, setSaveError] = useState(false);
  const settings = snapshot.settings;
  const adbQualityOnly = snapshot.sessionTransport === "aoa" ? false : snapshot.sessionConnectionTransport !== null
    ? snapshot.sessionConnectionTransport === "usb" && settings.wiredMode !== "aoa"
      || snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "adb"
    : snapshot.connectionTransport === "usb" ? settings.wiredMode !== "aoa" : settings.wirelessProtocol === "adb";
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
    ? `${negotiated.sampleRateHz / 1000} kHz · ${strings.app.audioSampleFormats[negotiated.sampleFormat]} · ${negotiated.channels} ch · ${negotiated.validBitsPerSample} ${strings.app.validBits}`
    : strings.app.audioFormatNotConfirmed;
  const mixFormat = mix
    ? `${mix.sampleRate === null ? strings.app.audioFormatUnknown : `${mix.sampleRate / 1000} kHz`} · ${mix.sampleFormat === null ? strings.app.audioFormatUnknown : strings.app.mixSampleFormats[mix.sampleFormat]} · ${mix.channels === null ? strings.app.audioFormatUnknown : `${mix.channels} ch`} · ${mix.containerBits === null ? strings.app.audioFormatUnknown : `${mix.containerBits} ${strings.app.containerBits}`} · ${mix.validBits === null ? strings.app.audioFormatUnknown : `${mix.validBits} ${strings.app.validBits}`}`
    : strings.app.audioFormatUnknown;
  const outputFormat = android
    ? `${android.sampleRate === null ? strings.app.audioFormatUnknown : `${android.sampleRate / 1000} kHz`} · ${android.encoding === null ? strings.app.audioFormatUnknown : strings.app.androidSampleFormats[android.encoding]} · ${android.channels === null ? strings.app.audioFormatUnknown : `${android.channels} ch`}`
    : strings.app.audioFormatUnknown;
  return <Modal title={strings.app.settingsTitle} onClose={onClose} className="settings-dialog">
    {(savingKeys.size > 0 || saveError) && <div className="settings-save-feedback">
      {savingKeys.size > 0 && <p className="dialog-footnote" role="status" aria-live="polite">{strings.app.settingsSavePending}</p>}
      {saveError && <p className="status-description actionable-error" role="alert">{strings.app.settingsSaveFailed}</p>}
    </div>}
    <section className="settings-group settings-appearance" aria-labelledby="settings-appearance-title">
      <h3 id="settings-appearance-title">{strings.app.settingsInterface}</h3>
      <label className="select-setting"><span>{strings.app.theme}</span><select disabled={snapshot.busy || savingKeys.has("appearanceTheme")} value={settings.appearanceTheme} onChange={event => void update("appearanceTheme", event.target.value)}><option value="System">{strings.app.followSystem}</option><option value="Light">{strings.app.light}</option><option value="Dark">{strings.app.dark}</option></select></label>
      <SettingToggle title={strings.app.enlargeText} description={strings.app.enlargeTextDescription} checked={largeText} onChange={onLargeText} />
      <SettingToggle disabled={snapshot.busy || savingKeys.has("minimizeToTray")} title={strings.app.minimizeToTray} description={strings.app.minimizeToTrayDescription} checked={settings.minimizeToTray} onChange={value => void update("minimizeToTray", value)} />
    </section>
    <section className="settings-audio-summary" aria-label={strings.app.settingsAudio}>
      <span>{strings.app.transportQuality}</span>
      <strong>{strings.app.audioQualityShortLabels[settings.audioQuality]}</strong>
    </section>
    <section className="settings-group settings-connection-shortcut" aria-labelledby="settings-connection-shortcut-title">
      <h3 id="settings-connection-shortcut-title">{strings.app.settingsConnection}</h3>
      <div className="settings-about-actions">
        <button type="button" disabled={busy} onClick={onOpenReceiverManager}><Smartphone size={15} aria-hidden="true" />{strings.app.phoneApplication}</button>
      </div>
    </section>
    <details className="settings-disclosure">
      <summary><span>{strings.app.advancedConnection}<small>{strings.app.advancedSettingsHint}</small></span></summary>
      <section className="settings-group settings-audio" aria-labelledby="settings-audio-title">
        <h3 id="settings-audio-title">{strings.app.settingsAudio}</h3>
        <fieldset className="audio-quality-settings" aria-describedby="audio-quality-help">
          <legend>{strings.app.transportQuality}</legend>
          <div className="audio-quality-options">
            {(["standard", "hifi", "hires"] as const).map(quality => {
              const fullLabel = strings.app.audioQualityLabels[quality];
              const requestedSpec = fullLabel.split(" · ").slice(1).join(" · ").replace(/\s*(?:（实验）|\(experimental\))$/i, "");
              return <label className="audio-quality-card" key={quality}>
                <input type="radio" name="audioQuality" value={quality} aria-label={fullLabel} checked={settings.audioQuality === quality} disabled={busy || savingKeys.has("audioQuality") || !snapshot.canChangeModes || adbQualityOnly && quality !== "standard"} onChange={() => void update("audioQuality", quality)} />
                <span>
                  <strong>{strings.app.audioQualityShortLabels[quality]}</strong>
                  <small>{strings.app.requestedSpecification}: {requestedSpec}</small>
                  {quality !== "standard" && <small>{strings.app.audioExperimental}</small>}
                </span>
              </label>;
            })}
          </div>
          {adbQualityOnly && <p className="dialog-footnote">{strings.app.adbStandardQualityOnly}</p>}
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
      </section>
      <section className="settings-group settings-connection" aria-labelledby="settings-connection-title">
        <h3 id="settings-connection-title">{strings.app.settingsConnection}</h3>
        <label className="select-setting"><span>{strings.app.wirelessProtocol}<small>{strings.app.wirelessProtocolHelp}</small></span><select aria-label={strings.app.wirelessProtocol} disabled={busy || savingKeys.has("wirelessProtocol")} value={settings.wirelessProtocol} onChange={event => { const value = event.target.value; void update("wirelessProtocol", value, () => onSetOption("wirelessProtocol", value)); }}><option value="udp">{strings.app.udpRecommended}</option><option value="adb">{strings.app.wirelessAdbCompatible}</option></select></label>
        <label className="select-setting"><span>{strings.app.receiverBackend}<small>{strings.app.receiverBackendHelp}</small></span><select aria-label={strings.app.receiverBackend} disabled={busy || udpBackendFixed || savingKeys.has("audioBackend")} value={settings.audioBackend} onChange={event => { const value = event.target.value; void update("audioBackend", value, () => onSetOption("audioBackend", value)); }}><option value="legacy">{strings.app.legacyCompatible}</option><option value="receiver">{strings.app.receiverModeRecommended}</option></select></label>
        {settings.wirelessProtocol === "adb" && <p className="dialog-footnote">{strings.app.wirelessAdbHelp}</p>}
        {aoaSession ? <p className="dialog-footnote">{strings.app.aoaReceiverBackendFixed}</p> : udpBackendFixed && <p className="dialog-footnote">{strings.app.udpUsesReceiver}</p>}
      </section>
    </details>
    <section className="settings-group settings-about" aria-labelledby="settings-about-title">
      <h3 id="settings-about-title">{strings.app.aboutAndDiagnostics}</h3>
      <div className="settings-about-actions">
        <button type="button" onClick={onOpenDiagnostics}><Activity size={15} aria-hidden="true" />{strings.app.diagnostics}</button>
        <button type="button" onClick={onOpenHelp}><CircleHelp size={15} aria-hidden="true" />{strings.app.help}</button>
      </div>
      {snapshot.busy && <p className="dialog-footnote">{strings.app.settingsBusy}</p>}
      <p className="dialog-footnote">{strings.app.settingsPrivacy}</p>
    </section>
  </Modal>;
}
