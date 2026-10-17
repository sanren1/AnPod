import { Check, Info, Search, Smartphone } from "lucide-react";
import { zhCN } from "../strings.zh-CN";
import type { Device } from "../appTypes";
import { bindingLabel } from "../appRuntime";
import { Modal } from "./Modal";

export function DeviceDialog({ devices, selected, activeSerial, activeBinding, busy, onSelect, onRefresh, onConnectOnly, onAddPhone, onFirstSetup, onClose }: { devices: Device[]; selected: string | null; activeSerial: string | null; activeBinding: string | null; busy: boolean; onSelect: (device: Device) => void; onRefresh: () => void; onConnectOnly: () => void; onAddPhone: (() => void) | null; onFirstSetup: (() => void) | null; onClose: () => void }) {
  return <Modal title={zhCN.deviceDialog.title} onClose={onClose} className="device-dialog">
    <div className="dialog-tools"><span>{zhCN.deviceDialog.onlyCurrentTransport}</span><button className="text-button" onClick={onRefresh} disabled={busy}><Search size={15} />{zhCN.deviceDialog.refresh}</button></div>
    {activeBinding && <div className="network-active-card"><strong>{bindingLabel(activeBinding)}</strong><span>{zhCN.deviceDialog.activeDirectUdp}</span></div>}
    <div className="device-list">{devices.map(device => { const connected = activeSerial === device.serial; const chosen = (activeSerial ?? selected) === device.serial; return <button className="device-option" key={device.serial} onClick={() => onSelect(device)} disabled={busy || (!connected && !device.connectable)} aria-pressed={chosen}>
      <Smartphone size={18} aria-hidden="true" /><span><strong>{device.displayName}</strong><small>{zhCN.deviceDialog.deviceDetails(connected, device.statusText, device.transport, device.androidVersion)}</small></span>{chosen && <Check size={17} aria-hidden="true" />}
    </button>; })}</div>
    {devices.length === 0 && <div className="empty-note"><Info size={17} /><span>{activeBinding ? zhCN.deviceDialog.emptyForBinding : zhCN.deviceDialog.empty}</span></div>}
    <div className="network-entry-actions">
      {devices.some(device => device.serial === selected && device.connectable) && <button className="text-button" type="button" disabled={busy} onClick={onConnectOnly}>{zhCN.app.connectOnly}</button>}
      {onAddPhone && <button className="text-button" type="button" disabled={busy} onClick={onAddPhone}>{zhCN.app.addPhone}</button>}
      {onFirstSetup && <button className="text-button" type="button" disabled={busy} onClick={onFirstSetup}>{zhCN.app.firstSetup}</button>}
    </div>
    <button className="dialog-secondary" onClick={onClose}>{zhCN.deviceDialog.finish}</button>
  </Modal>;
}
