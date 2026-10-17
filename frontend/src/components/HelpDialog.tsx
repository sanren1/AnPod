import { zhCN } from "../strings.zh-CN";
import type { Snapshot } from "../appTypes";
import { Modal } from "./Modal";

export function HelpDialog({ transport, wirelessProtocol, onClose }: { transport: Snapshot["connectionTransport"]; wirelessProtocol: Snapshot["sessionWirelessProtocol"]; onClose: () => void }) {
  return <Modal title={zhCN.helpDialog.title} onClose={onClose} className="help-dialog">
    {transport === "usb" ? <><h3>{zhCN.helpDialog.usbTitle}</h3><ol><li>{zhCN.helpDialog.usbStep1}</li><li>{zhCN.helpDialog.usbStep2}</li></ol></>
      : wirelessProtocol === "udp" ? <><h3>{zhCN.helpDialog.udpTitle}</h3><ol><li>{zhCN.helpDialog.udpStep1}</li><li>{zhCN.helpDialog.udpStep2}</li><li>{zhCN.helpDialog.udpStep3}</li></ol></>
        : <><h3>{zhCN.helpDialog.adbTitle}</h3><ol><li>{zhCN.helpDialog.adbStep1}</li><li>{zhCN.helpDialog.adbStep2}</li></ol></>}
    <p>{transport === "usb" ? zhCN.helpDialog.usbPlaybackNote
      : wirelessProtocol === "udp" ? zhCN.helpDialog.receiverRestartNote
        : zhCN.helpDialog.adbPlaybackNote}</p><button className="dialog-secondary" onClick={onClose}>{zhCN.helpDialog.finish}</button>
  </Modal>;
}
