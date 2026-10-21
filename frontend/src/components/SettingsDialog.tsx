import { strings } from "../strings";
import type { Settings, Snapshot } from "../appTypes";
import { Modal } from "./Modal";
import { SettingToggle } from "./SettingToggle";

export function SettingsDialog({ snapshot, largeText, onLargeText, udpBackendFixed, aoaSession, busy, onSetOption, onOpenReceiverManager, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; udpBackendFixed: boolean; aoaSession: boolean; busy: boolean; onSetOption: (kind: "wirelessProtocol" | "audioBackend", value: string) => void; onOpenReceiverManager: () => void; onSave: (key: string, value: unknown) => void; onClose: () => void }) {
  const settings = snapshot.settings;
  const adbQualityOnly = snapshot.sessionTransport === "aoa" ? false : snapshot.sessionConnectionTransport !== null
    ? snapshot.sessionConnectionTransport === "usb" && settings.wiredMode !== "aoa"
      || snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "adb"
    : snapshot.connectionTransport === "usb" ? settings.wiredMode !== "aoa" : settings.wirelessProtocol === "adb";
  const update = (key: keyof Settings, value: boolean | string) => { void onSave(key, value); };
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
    <fieldset className="audio-quality-settings" aria-describedby="audio-quality-help">
      <legend>{strings.app.transportQuality}</legend>
      {(["standard", "hifi", "hires"] as const).map(quality => <label className="setting-toggle" key={quality}>
        <span>{strings.app.audioQualityLabels[quality]}</span>
        <input type="radio" name="audioQuality" value={quality} checked={settings.audioQuality === quality} disabled={busy || !snapshot.canChangeModes || adbQualityOnly && quality !== "standard"} onChange={() => update("audioQuality", quality)} />
      </label>)}
      {adbQualityOnly && <p className="dialog-footnote">{strings.app.adbStandardQualityOnly}</p>}
      <p id="audio-quality-help" className="dialog-footnote">{strings.app.audioQualityInDevelopment}</p>
      <dl className="audio-quality-status">
        <div><dt>{strings.app.actualTransportFormat}</dt><dd>{actualFormat}</dd></div>
        <div><dt>{strings.app.windowsMixFormat}</dt><dd>{mixFormat}</dd></div>
        <div><dt>{strings.app.androidOutputFormat}</dt><dd>{outputFormat}</dd></div>
      </dl>
      <p className="dialog-footnote">{strings.app.audioOutputPrecisionNotice}</p>
    </fieldset>
    <section className="settings-advanced" aria-labelledby="settings-advanced-title">
      <h3 id="settings-advanced-title">{strings.app.advancedConnection}</h3>
      <label className="select-setting"><span>{strings.app.wirelessProtocol}</span><select aria-label={strings.app.wirelessProtocol} disabled={busy} value={settings.wirelessProtocol} onChange={event => onSetOption("wirelessProtocol", event.target.value)}><option value="udp">{strings.app.udpRecommended}</option><option value="adb">{strings.app.wirelessAdbCompatible}</option></select></label>
      <label className="select-setting"><span>{strings.app.receiverBackend}</span><select aria-label={strings.app.receiverBackend} disabled={busy || udpBackendFixed} value={settings.audioBackend} onChange={event => onSetOption("audioBackend", event.target.value)}><option value="legacy">{strings.app.legacyCompatible}</option><option value="receiver">{strings.app.receiverModeRecommended}</option></select></label>
      {settings.wirelessProtocol === "adb" && <p className="dialog-footnote">{strings.app.wirelessAdbHelp}</p>}
      {aoaSession ? <p className="dialog-footnote">{strings.app.aoaReceiverBackendFixed}</p> : udpBackendFixed && <p className="dialog-footnote">{strings.app.udpUsesReceiver}</p>}
    </section>
    <button className="text-button backend-manager-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>{strings.app.phoneApplication}</button>
    <h3>{strings.app.windowAppearance}</h3>
    <SettingToggle disabled={snapshot.busy} title={strings.app.minimizeToTray} description={strings.app.minimizeToTrayDescription} checked={settings.minimizeToTray} onChange={value => update("minimizeToTray", value)} />
    <SettingToggle title={strings.app.enlargeText} description={strings.app.enlargeTextDescription} checked={largeText} onChange={onLargeText} />
    <label className="select-setting"><span>{strings.app.theme}</span><select disabled={snapshot.busy} value={settings.appearanceTheme} onChange={event => update("appearanceTheme", event.target.value)}><option value="System">{strings.app.followSystem}</option><option value="Light">{strings.app.light}</option><option value="Dark">{strings.app.dark}</option></select></label>
    {snapshot.busy && <p className="dialog-footnote">{strings.app.settingsBusy}</p>}
    <p className="dialog-footnote">{strings.app.settingsPrivacy}</p>
  </Modal>;
}
