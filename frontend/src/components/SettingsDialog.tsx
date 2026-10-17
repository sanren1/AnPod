import { zhCN } from "../strings.zh-CN";
import type { Settings, Snapshot } from "../appTypes";
import { Modal } from "./Modal";
import { SettingToggle } from "./SettingToggle";

export function SettingsDialog({ snapshot, largeText, onLargeText, udpBackendFixed, busy, onSetOption, onOpenReceiverManager, onSave, onClose }: { snapshot: Snapshot; largeText: boolean; onLargeText: (value: boolean) => void; udpBackendFixed: boolean; busy: boolean; onSetOption: (kind: "wirelessProtocol" | "audioBackend", value: string) => void; onOpenReceiverManager: () => void; onSave: (key: string, value: unknown) => void; onClose: () => void }) {
  const settings = snapshot.settings;
  const update = (key: keyof Settings, value: boolean | string) => { void onSave(key, value); };
  return <Modal title={zhCN.app.settingsTitle} onClose={onClose} className="settings-dialog">
    <h3>{zhCN.app.settingsConnection}</h3>
    <SettingToggle disabled={snapshot.busy} title={zhCN.app.autoConnect} description={zhCN.app.autoConnectDescription} checked={settings.autoConnectLastDevice} onChange={value => update("autoConnectLastDevice", value)} />
    <SettingToggle disabled={snapshot.busy} title={zhCN.app.wirelessRecovery} description={zhCN.app.wirelessRecoveryDescription} checked={settings.autoReconnectWifi} onChange={value => update("autoReconnectWifi", value)} />
    <details className="settings-advanced">
      <summary>{zhCN.app.advancedConnection}</summary>
      <label className="select-setting"><span>{zhCN.app.wirelessProtocol}</span><select aria-label={zhCN.app.wirelessProtocol} disabled={busy} value={settings.wirelessProtocol} onChange={event => onSetOption("wirelessProtocol", event.target.value)}><option value="udp">{zhCN.app.udpRecommended}</option><option value="adb">{zhCN.app.wirelessAdbCompatible}</option></select></label>
      <label className="select-setting"><span>{zhCN.app.receiverBackend}</span><select aria-label={zhCN.app.receiverBackend} disabled={busy || udpBackendFixed} value={settings.audioBackend} onChange={event => onSetOption("audioBackend", event.target.value)}><option value="legacy">{zhCN.app.legacyCompatible}</option><option value="receiver">Receiver</option></select></label>
      {settings.wirelessProtocol === "adb" && <p className="dialog-footnote">{zhCN.app.wirelessAdbHelp}</p>}
      {udpBackendFixed && <p className="dialog-footnote">{zhCN.app.udpUsesReceiver}</p>}
      <button className="text-button backend-manager-button" type="button" disabled={busy} onClick={onOpenReceiverManager}>{zhCN.app.phoneApplication}</button>
    </details>
    <h3>{zhCN.app.windowAppearance}</h3>
    <SettingToggle disabled={snapshot.busy} title={zhCN.app.minimizeToTray} description={zhCN.app.minimizeToTrayDescription} checked={settings.minimizeToTray} onChange={value => update("minimizeToTray", value)} />
    <SettingToggle title={zhCN.app.enlargeText} description={zhCN.app.enlargeTextDescription} checked={largeText} onChange={onLargeText} />
    <label className="select-setting"><span>{zhCN.app.theme}</span><select disabled={snapshot.busy} value={settings.appearanceTheme} onChange={event => update("appearanceTheme", event.target.value)}><option value="System">{zhCN.app.followSystem}</option><option value="Light">{zhCN.app.light}</option><option value="Dark">{zhCN.app.dark}</option></select></label>
    {snapshot.busy && <p className="dialog-footnote">{zhCN.app.settingsBusy}</p>}
    <p className="dialog-footnote">{zhCN.app.settingsPrivacy}</p>
  </Modal>;
}
