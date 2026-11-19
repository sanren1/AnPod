import type { Ref } from "react";
import { LoaderCircle } from "lucide-react";
import { strings } from "../strings";
import type { ConnectionOffer } from "../appTypes";
import { ErrorDetails } from "./Modal";

export type DeviceRowError = { summary: string; details: string };

export function SavedDeviceRow({ label, online, offer, error, errorRef, busy, connecting, onConnect, onAccept, onDecline }: {
  label: string; online: boolean; offer: ConnectionOffer | null; error: DeviceRowError | null; errorRef?: Ref<HTMLParagraphElement>;
  busy: boolean; connecting: boolean; onConnect: () => void; onAccept: () => void; onDecline: () => void;
}) {
  const status = offer ? strings.network.deviceRequest : online ? strings.network.deviceOnline : strings.network.deviceNotFound;
  return <li className={`device-row${offer ? " is-request" : ""}${error && !offer ? " is-failed" : ""}`}>
    <div className="device-row-main">
      <span className="device-row-dot" data-online={online || undefined} aria-hidden="true" />
      <div className="device-row-copy"><strong>{label}</strong><small>{status}</small></div>
      {offer
        ? <div className="device-row-actions">
          <button className="dialog-secondary" type="button" disabled={busy} onClick={onDecline}>{strings.network.declineRequest}</button>
          <button className="primary-button" type="button" disabled={busy} onClick={onAccept}>{strings.network.acceptRequest}</button>
        </div>
        : <button className={online ? "primary-button" : "dialog-secondary"} type="button" disabled={busy} onClick={onConnect}>
          {connecting ? <><LoaderCircle className="spin" size={15} />{strings.network.connectingDevice}</> : error ? strings.network.retryConnect : strings.network.connectDevice}
        </button>}
    </div>
    {error && (error.details
      ? <ErrorDetails className="device-row-error" summary={error.summary} details={error.details} summaryRef={errorRef} />
      : <p className="inline-error device-row-error" role="alert" tabIndex={-1} ref={errorRef}>{error.summary}</p>)}
  </li>;
}
