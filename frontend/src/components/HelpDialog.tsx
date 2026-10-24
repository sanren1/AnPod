import { strings } from "../strings";
import type { Snapshot } from "../appTypes";
import { Modal } from "./Modal";

export function HelpDialog({ transport, wirelessProtocol, onClose, returnFocusTarget }: { transport: Snapshot["connectionTransport"]; wirelessProtocol: Snapshot["sessionWirelessProtocol"]; onClose: () => void; returnFocusTarget?: () => HTMLElement | null }) {
  return <Modal title={strings.helpDialog.title} onClose={onClose} className="help-dialog" returnFocusTarget={returnFocusTarget}>
    {transport === "usb" ? <><h3>{strings.helpDialog.usbTitle}</h3><ol><li>{strings.helpDialog.usbStep1}</li><li>{strings.helpDialog.usbStep2}</li></ol></>
      : wirelessProtocol === "udp" ? <><h3>{strings.helpDialog.udpTitle}</h3><ol><li>{strings.helpDialog.udpStep1}</li><li>{strings.helpDialog.udpStep2}</li><li>{strings.helpDialog.udpStep3}</li></ol></>
        : <><h3>{strings.helpDialog.adbTitle}</h3><ol><li>{strings.helpDialog.adbStep1}</li><li>{strings.helpDialog.adbStep2}</li></ol></>}
    <p>{transport === "usb" ? strings.helpDialog.usbPlaybackNote
      : wirelessProtocol === "udp" ? strings.helpDialog.receiverRestartNote
        : strings.helpDialog.adbPlaybackNote}</p><button className="dialog-secondary" onClick={onClose}>{strings.helpDialog.finish}</button>
  </Modal>;
}
