import { strings } from "../strings";
import type { Settings, Snapshot } from "../appTypes";
import { Modal } from "./Modal";
import { SettingToggle } from "./SettingToggle";

export function SettingsDialog({ snapshot, largeText, onLargeText, udpBackendFixed, busy, onSetOption, onOpenReceiverManager, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; udpBackendFixed: boolean; busy: boolean; onSetOption: (kind: "wirelessProtocol" | "audioBackend", value: string) => void; onOpenReceiverManager: () => void; onSave: (key: string, value: unknown) => void; onClose: () => void }) {
  const settings = snapshot.settings;
  const update = (key: keyof Settings, value: boolean | string) => { void onSave(key, value); };
  return <Modal title={strings.app.settingsTitle} onClose={onClose} className="settings-dialog">
    <h3>{strings.app.settingsConnection}</h3>
    <SettingToggle disabled={snapshot.busy} title={strings.app.autoConnect} description={strings.app.autoConnectDescription} checked={settings.autoConnectLastDevice} onChange={value => update("autoConnectLastDevice", value)} />
    <SettingToggle disabled={snapshot.busy} title={strings.app.wirelessRecovery} description={strings.app.wirelessRecoveryDescription} checked={settings.autoReconnectWifi} onChange={value => update("autoReconnectWifi", value)} />
    <section className="settings-advanced" aria-labelledby="settings-advanced-title">
      <h3 id="settings-advanced-title">{strings.app.advancedConnection}</h3>
      <label className="select-setting"><span>{strings.app.wirelessProtocol}</span><select aria-label={strings.app.wirelessProtocol} disabled={busy} value={settings.wirelessProtocol} onChange={event => onSetOption("wirelessProtocol", event.target.value)}><option value="udp">{strings.app.udpRecommended}</option><option value="adb">{strings.app.wirelessAdbCompatible}</option></select></label>
      <label className="select-setting"><span>{strings.app.receiverBackend}</span><select aria-label={strings.app.receiverBackend} disabled={busy || udpBackendFixed} value={settings.audioBackend} onChange={event => onSetOption("audioBackend", event.target.value)}><option value="legacy">{strings.app.legacyCompatible}</option><option value="receiver">{strings.app.receiverModeRecommended}</option></select></label>
      {settings.wirelessProtocol === "adb" && <p className="dialog-footnote">{strings.app.wirelessAdbHelp}</p>}
      {udpBackendFixed && <p className="dialog-footnote">{strings.app.udpUsesReceiver}</p>}
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
