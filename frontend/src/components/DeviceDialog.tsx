import { Check, Info, Search, Smartphone } from "lucide-react";
import { strings } from "../strings";
import type { Device } from "../appTypes";
import { bindingLabel } from "../appRuntime";
import { Modal } from "./Modal";
import { FluidSegmentedControl } from "./ui/FluidSegmentedControl";

export function DeviceDialog({ devices, selected, active, activeSerial, activeBinding, connectionTransport, wiredMode, wirelessProtocol, busy, onSelect, onRefresh, onConnectOnly, onAddPhone, onFirstSetup, onOpenDirect, onOpenUdp, onSetOption, onSetWiredMode, onClose }: { devices: Device[]; selected: string | null; active: boolean; activeSerial: string | null; activeBinding: string | null; connectionTransport: "usb" | "wireless"; wiredMode: "adb" | "aoa"; wirelessProtocol: "udp" | "adb"; busy: boolean; onSelect: (device: Device) => void; onRefresh: () => void; onConnectOnly: () => void; onAddPhone: (() => void) | null; onFirstSetup: (() => void) | null; onOpenDirect: () => void; onOpenUdp: () => void; onSetOption: (kind: "transport" | "wirelessProtocol", value: string) => void; onSetWiredMode: (mode: "adb" | "aoa") => void; onClose: () => void }) {
  const isDirect = connectionTransport === "usb" && wiredMode === "aoa";
  const isUdp = connectionTransport === "wireless" && wirelessProtocol === "udp";
  return <Modal title={strings.deviceDialog.title} onClose={onClose} className="device-dialog">
    <div className="device-connection-choice">
      <span className="dialog-section-label">{strings.app.connectionMode}</span>
      <FluidSegmentedControl label={strings.app.connectionMode} value={connectionTransport} className="transport-switch" onValueChange={value => onSetOption("transport", value)} options={[
        { value: "usb", label: strings.app.usb, disabled: busy },
        { value: "wireless", label: strings.app.wireless, disabled: busy },
      ]} />
      {connectionTransport === "usb"
        ? <FluidSegmentedControl label={strings.app.wiredMode} value={wiredMode} className="transport-switch" onValueChange={onSetWiredMode} options={[
          { value: "adb", label: strings.app.wiredModeAdb, disabled: busy },
          { value: "aoa", label: strings.app.wiredModeDirect, disabled: busy },
        ]} />
        : <FluidSegmentedControl label={strings.app.wirelessProtocol} value={wirelessProtocol} className="transport-switch" onValueChange={value => onSetOption("wirelessProtocol", value)} options={[
          { value: "udp", label: strings.app.wirelessUdp, disabled: busy },
          { value: "adb", label: strings.app.wirelessAdb, disabled: busy },
        ]} />}
    </div>
    {!isDirect && !isUdp && <div className="dialog-tools"><span>{strings.deviceDialog.onlyCurrentTransport}</span><button className="text-button" onClick={onRefresh} disabled={busy}><Search size={15} />{strings.deviceDialog.refresh}</button></div>}
    {activeBinding && <div className="network-active-card"><strong>{bindingLabel(activeBinding)}</strong><span>{strings.deviceDialog.activeDirectUdp}</span></div>}
    {isDirect && !active && <button className="primary-button" type="button" disabled={busy} onClick={onOpenDirect}>{strings.app.connectUsbDirect}</button>}
    {isUdp && !active && <button className="primary-button" type="button" disabled={busy} onClick={onOpenUdp}>{strings.app.choosePairedPhone}</button>}
    {!isDirect && !isUdp && <div className="device-list">{devices.map(device => { const connected = activeSerial === device.serial; const chosen = (activeSerial ?? selected) === device.serial; return <button className="device-option" key={device.serial} onClick={() => onSelect(device)} disabled={busy || (!connected && !device.connectable)} aria-pressed={chosen}>
      <Smartphone size={18} aria-hidden="true" /><span><strong>{device.displayName}</strong><small>{strings.deviceDialog.deviceDetails(connected, device.statusText, device.transport, device.androidVersion)}</small></span>{chosen && <Check size={17} aria-hidden="true" />}
    </button>; })}</div>}
    {!isDirect && !isUdp && devices.length === 0 && <div className="empty-note"><Info size={17} /><span>{activeBinding ? strings.deviceDialog.emptyForBinding : strings.deviceDialog.empty}</span></div>}
    <div className="network-entry-actions">
      {!isDirect && !isUdp && devices.some(device => device.serial === selected && device.connectable) && <button className="text-button" type="button" disabled={busy} onClick={onConnectOnly}>{strings.app.connectOnly}</button>}
      {onAddPhone && <button className="text-button" type="button" disabled={busy} onClick={onAddPhone}>{strings.app.addPhone}</button>}
      {onFirstSetup && <button className="text-button" type="button" disabled={busy} onClick={onFirstSetup}>{strings.app.firstSetup}</button>}
    </div>
    <button className="dialog-secondary" onClick={onClose}>{strings.deviceDialog.finish}</button>
  </Modal>;
}
