import { useState } from "react";
import { Copy, Download, ExternalLink } from "lucide-react";
import { strings } from "../strings";
import type { Snapshot } from "../appTypes";
import { invokeApp, hasActiveSession, intendedConnectionLabel, actualConnectionLabel, bindingReferenceLabel, playbackModeLabel, readableStatus, CAP_PLAYBACK_PROGRESS } from "../appRuntime";
import { Modal } from "./Modal";
import { ErrorDetails } from "./Modal";

export function DiagnosticsDialog({ snapshot, onClose, returnFocusTarget }: { snapshot: Snapshot; onClose: () => void; returnFocusTarget?: () => HTMLElement | null }) {
  const capabilities = snapshot.sessionCapabilities;
  const diagnostic = snapshot.playbackDiagnostics;
  const [exporting, setExporting] = useState(false);
  const [exportPath, setExportPath] = useState<string | null>(null);
  const [exportError, setExportError] = useState("");
  const exportSnapshot = async () => {
    setExporting(true); setExportError(""); setExportPath(null);
    try { setExportPath(await invokeApp<string>("export_diagnostics")); }
    catch (error) { setExportError(error instanceof Error ? error.message : String(error)); }
    finally { setExporting(false); }
  };
  const active = hasActiveSession(snapshot);
  const udpSession = active && snapshot.sessionTransport === "lan"
    && snapshot.sessionConnectionTransport === "wireless" && snapshot.sessionWirelessProtocol === "udp";
  const unsupportedUdpDiagnostics = udpSession
    && (capabilities === null || (capabilities.capabilityBits & CAP_PLAYBACK_PROGRESS) === 0);
  const number = (value: number | null) => value === null ? strings.diagnostics.unavailable : value.toLocaleString("zh-CN");
  const frames = (value: number | null, meaning: string) => value === null ? strings.diagnostics.unavailable : strings.diagnostics.frameValue(number(value), meaning);
  const playbackContext = !active ? strings.diagnostics.noPhoneSession
    : snapshot.output === "computer" ? strings.diagnostics.computerSnapshot
      : snapshot.state === "paused" ? strings.diagnostics.pausedSnapshot
        : diagnostic?.underrunIncrement !== null && diagnostic?.underrunIncrement !== undefined && diagnostic.underrunIncrement > 0
          ? strings.diagnostics.underrunDelta(number(diagnostic.underrunIncrement))
        : diagnostic?.playbackAdvancing === true ? strings.diagnostics.progressConfirmed
          : diagnostic?.playbackAdvancing === false ? strings.diagnostics.progressNotConfirmed
            : unsupportedUdpDiagnostics ? strings.diagnostics.unsupportedProgress
              : strings.diagnostics.missingProgress;
  const unavailable = !active ? strings.diagnostics.noSessionSnapshot
    : unsupportedUdpDiagnostics ? strings.diagnostics.unsupportedDiagnostics
      : strings.diagnostics.missingDiagnostics;
  const groups: { title: string; note?: string; metrics: [string, string][] }[] = diagnostic ? [
    { title: strings.diagnostics.actualAudioFormat, metrics: [
      [strings.diagnostics.pcSampleRate, diagnostic.sampleRate === null ? strings.diagnostics.unavailable : `${number(diagnostic.sampleRate)} Hz`],
      [strings.diagnostics.pcChannels, number(diagnostic.channels)],
      [strings.diagnostics.receiverSampleRate, diagnostic.receiverSampleRate === null ? strings.diagnostics.unavailable : `${number(diagnostic.receiverSampleRate)} Hz`],
      [strings.diagnostics.receiverChannels, number(diagnostic.receiverChannels)],
    ] },
    { title: strings.diagnostics.queues, note: strings.diagnostics.queueNote, metrics: [
      [strings.diagnostics.pcQueue, frames(diagnostic.pcQueuedFrames, strings.diagnostics.currentQueueMeaning)],
      [strings.diagnostics.pcDropped, frames(diagnostic.pcDroppedFrames, strings.diagnostics.sessionTotalMeaning)],
      [strings.diagnostics.udpJitterTarget, frames(diagnostic.udpJitterTargetFrames, strings.diagnostics.targetQueueMeaning)],
      [strings.diagnostics.udpQueue, frames(diagnostic.udpQueuedFrames, strings.diagnostics.currentQueueMeaning)],
      [strings.diagnostics.udpExpired, frames(diagnostic.udpExpiredFrames, strings.diagnostics.sessionTotalMeaning)],
      [strings.diagnostics.tcpDropped, frames(diagnostic.tcpDroppedBacklogFrames, strings.diagnostics.sessionTotalMeaning)],
    ] },
    { title: strings.diagnostics.receiverPlayback, note: strings.diagnostics.playbackNote, metrics: [
      [strings.diagnostics.audioTrackPending, frames(diagnostic.trackPendingFrames, strings.diagnostics.currentQueryMeaning)],
      [strings.diagnostics.underrunTotal, diagnostic.underrunCount === null ? strings.diagnostics.unavailable : `${number(diagnostic.underrunCount)} · ${strings.diagnostics.audioTrackCumulativeMeaning}`],
      [strings.diagnostics.underrunIncrement, diagnostic.underrunIncrement === null ? strings.diagnostics.unavailable : `${number(diagnostic.underrunIncrement)} · ${strings.diagnostics.sinceLastQueryMeaning}`],
      [strings.diagnostics.advancing, diagnostic.playbackAdvancing === null ? strings.diagnostics.unavailable : diagnostic.playbackAdvancing ? strings.diagnostics.yes : strings.diagnostics.no],
    ] },
    { title: strings.diagnostics.audioTrackBuffer, note: strings.diagnostics.bufferNote, metrics: [
      [strings.diagnostics.requested, frames(diagnostic.requestedBufferFrames, strings.diagnostics.audioTrackMeaning)],
      [strings.diagnostics.accepted, frames(diagnostic.acceptedBufferFrames, strings.diagnostics.audioTrackMeaning)],
      [strings.diagnostics.effective, frames(diagnostic.effectiveBufferFrames, strings.diagnostics.audioTrackMeaning)],
      [strings.diagnostics.capacity, frames(diagnostic.capacityFrames, strings.diagnostics.audioTrackMeaning)],
      [strings.diagnostics.startThreshold, frames(diagnostic.startThresholdFrames, strings.diagnostics.audioTrackMeaning)],
      [strings.diagnostics.preRoll, frames(diagnostic.preRollFrames, strings.diagnostics.audioTrackMeaning)],
    ] },
  ] : [];
  const statusText = [
    `${strings.diagnostics.state}：${readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}`,
    `${strings.diagnostics.serviceMessage}：${snapshot.message}`, `${strings.diagnostics.actualConnection}：${active ? actualConnectionLabel(snapshot) : strings.diagnostics.notConnected}`,
    `${strings.diagnostics.chosenConnection}：${intendedConnectionLabel(snapshot)}`, `${strings.diagnostics.receiverBackend}：${snapshot.settings.audioBackend}`,
    `${strings.diagnostics.device}：${snapshot.sessionReceiverBinding ? bindingReferenceLabel(snapshot.sessionReceiverBinding) : snapshot.deviceName ?? strings.diagnostics.none}`,
    `${strings.diagnostics.output}：${snapshot.output === "phone" ? strings.diagnostics.phone : strings.diagnostics.computer}`, `${strings.diagnostics.visibleDevices}：${snapshot.devices.length}`,
    `${strings.diagnostics.detail}：${snapshot.statusDetail}`,
    `${strings.diagnostics.capabilities}：${capabilities ? strings.diagnostics.protocolCompatibility(capabilities.controlMajor, capabilities.managementCompatible, capabilities.audioMajor, capabilities.audioCompatible, capabilities.capabilityBits.toString(16), capabilities.versionCode) : strings.diagnostics.unknownCapabilities}`,
    `${strings.diagnostics.version}：${snapshot.version}`,
  ].join("\n");
  const copyText = [
    `${strings.diagnostics.state}：${readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}`,
    `${strings.diagnostics.actualConnection}：${active ? actualConnectionLabel(snapshot) : strings.diagnostics.notConnected}`,
    `${strings.diagnostics.playbackMode}：${playbackModeLabel(snapshot.settings.playbackMode)}`,
    `${strings.diagnostics.output}：${snapshot.output === "phone" ? strings.diagnostics.phone : strings.diagnostics.computer}`,
    `${strings.diagnostics.version}：${snapshot.version}`,
    `${strings.diagnostics.capabilitySummary}：${capabilities ? strings.diagnostics.protocolCompatibility(capabilities.controlMajor, capabilities.managementCompatible, capabilities.audioMajor, capabilities.audioCompatible, capabilities.capabilityBits.toString(16), capabilities.versionCode) : strings.diagnostics.unknownCapabilities}`,
    `${strings.diagnostics.playbackDiagnostics}：${playbackContext}`,
    ...groups.flatMap(group => [group.title, ...group.metrics.map(([label, value]) => `${label}：${value}`), ...(group.note ? [group.note] : [])]),
  ].join("\n");
  const metricGroup = (group: typeof groups[number]) => <section className="diagnostic-group" key={group.title}>
    <h3>{group.title}</h3>
    <dl>{group.metrics.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
    {group.note && <p>{group.note}</p>}
  </section>;
  return <Modal title={strings.diagnostics.dialogTitle} onClose={onClose} className="diagnostics-dialog" returnFocusTarget={returnFocusTarget}>
    <section className="diagnostic-overview" aria-label={strings.diagnostics.currentState}>
      <dl><div><dt>{strings.diagnostics.state}</dt><dd>{readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}</dd></div>
        <div><dt>{strings.diagnostics.actualConnection}</dt><dd>{active ? actualConnectionLabel(snapshot) : strings.diagnostics.notConnected}</dd></div>
        <div><dt>{strings.diagnostics.playbackMode}</dt><dd>{playbackModeLabel(snapshot.settings.playbackMode)}</dd></div></dl>
      <p className="diagnostic-context" role="status">{playbackContext}</p>
    </section>
    <details className="diagnostic-details">
      <summary>{strings.diagnostics.technicalDetails}</summary>
      <p>{strings.diagnostics.runState}</p><pre>{statusText}</pre>
      <section className="diagnostic-playback" aria-label={strings.diagnostics.playbackDiagnostics}>
        <h3>{strings.diagnostics.playbackDiagnostics}</h3>
        {diagnostic ? groups.map(metricGroup) : <p className="diagnostic-empty">{unavailable}</p>}
      </section>
      <div className="dialog-actions"><button className="dialog-secondary" onClick={() => void navigator.clipboard.writeText(copyText)}><Copy size={15} />{strings.diagnostics.copyDiagnostics}</button><button className="dialog-secondary" disabled={exporting} onClick={() => void exportSnapshot()}><Download size={15} />{exporting ? strings.diagnostics.exporting : strings.diagnostics.exportDiagnostics}</button><button className="dialog-secondary" onClick={() => void invokeApp("open_logs")}>{strings.diagnostics.openLogs}<ExternalLink size={14} /></button></div>
      {exportPath && <p className="diagnostic-context" role="status">{strings.diagnostics.exportSaved(exportPath)}</p>}
      {exportError && <ErrorDetails summary={strings.diagnostics.exportFailed} details={exportError} />}
    </details>
  </Modal>;
}
