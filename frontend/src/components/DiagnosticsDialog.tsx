import { useState } from "react";
import { Copy, Download, ExternalLink } from "lucide-react";
import { zhCN } from "../strings.zh-CN";
import type { Snapshot } from "../appTypes";
import { invokeApp, hasActiveSession, intendedConnectionLabel, actualConnectionLabel, bindingReferenceLabel, playbackModeLabel, readableStatus, CAP_PLAYBACK_PROGRESS } from "../appRuntime";
import { Modal } from "./Modal";
import { ErrorDetails } from "./Modal";

export function DiagnosticsDialog({ snapshot, onClose }: { snapshot: Snapshot; onClose: () => void }) {
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
  const number = (value: number | null) => value === null ? zhCN.diagnostics.unavailable : value.toLocaleString("zh-CN");
  const frames = (value: number | null, meaning: string) => value === null ? zhCN.diagnostics.unavailable : zhCN.diagnostics.frameValue(number(value), meaning);
  const playbackContext = !active ? zhCN.diagnostics.noPhoneSession
    : snapshot.output === "computer" ? zhCN.diagnostics.computerSnapshot
      : snapshot.state === "paused" ? zhCN.diagnostics.pausedSnapshot
        : diagnostic?.underrunIncrement !== null && diagnostic?.underrunIncrement !== undefined && diagnostic.underrunIncrement > 0
          ? zhCN.diagnostics.underrunDelta(number(diagnostic.underrunIncrement))
        : diagnostic?.playbackAdvancing === true ? zhCN.diagnostics.progressConfirmed
          : diagnostic?.playbackAdvancing === false ? zhCN.diagnostics.progressNotConfirmed
            : unsupportedUdpDiagnostics ? zhCN.diagnostics.unsupportedProgress
              : zhCN.diagnostics.missingProgress;
  const unavailable = !active ? zhCN.diagnostics.noSessionSnapshot
    : unsupportedUdpDiagnostics ? zhCN.diagnostics.unsupportedDiagnostics
      : zhCN.diagnostics.missingDiagnostics;
  const groups: { title: string; note?: string; metrics: [string, string][] }[] = diagnostic ? [
    { title: zhCN.diagnostics.actualAudioFormat, metrics: [
      [zhCN.diagnostics.pcSampleRate, diagnostic.sampleRate === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.sampleRate)} Hz`],
      [zhCN.diagnostics.pcChannels, number(diagnostic.channels)],
      [zhCN.diagnostics.receiverSampleRate, diagnostic.receiverSampleRate === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.receiverSampleRate)} Hz`],
      [zhCN.diagnostics.receiverChannels, number(diagnostic.receiverChannels)],
    ] },
    { title: zhCN.diagnostics.queues, note: zhCN.diagnostics.queueNote, metrics: [
      [zhCN.diagnostics.pcQueue, frames(diagnostic.pcQueuedFrames, zhCN.diagnostics.currentQueueMeaning)],
      [zhCN.diagnostics.pcDropped, frames(diagnostic.pcDroppedFrames, zhCN.diagnostics.sessionTotalMeaning)],
      [zhCN.diagnostics.udpJitterTarget, frames(diagnostic.udpJitterTargetFrames, zhCN.diagnostics.targetQueueMeaning)],
      [zhCN.diagnostics.udpQueue, frames(diagnostic.udpQueuedFrames, zhCN.diagnostics.currentQueueMeaning)],
      [zhCN.diagnostics.udpExpired, frames(diagnostic.udpExpiredFrames, zhCN.diagnostics.sessionTotalMeaning)],
      [zhCN.diagnostics.tcpDropped, frames(diagnostic.tcpDroppedBacklogFrames, zhCN.diagnostics.sessionTotalMeaning)],
    ] },
    { title: zhCN.diagnostics.receiverPlayback, note: zhCN.diagnostics.playbackNote, metrics: [
      [zhCN.diagnostics.audioTrackPending, frames(diagnostic.trackPendingFrames, zhCN.diagnostics.currentQueryMeaning)],
      [zhCN.diagnostics.underrunTotal, diagnostic.underrunCount === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.underrunCount)} · ${zhCN.diagnostics.audioTrackCumulativeMeaning}`],
      [zhCN.diagnostics.underrunIncrement, diagnostic.underrunIncrement === null ? zhCN.diagnostics.unavailable : `${number(diagnostic.underrunIncrement)} · ${zhCN.diagnostics.sinceLastQueryMeaning}`],
      [zhCN.diagnostics.advancing, diagnostic.playbackAdvancing === null ? zhCN.diagnostics.unavailable : diagnostic.playbackAdvancing ? zhCN.diagnostics.yes : zhCN.diagnostics.no],
    ] },
    { title: zhCN.diagnostics.audioTrackBuffer, note: zhCN.diagnostics.bufferNote, metrics: [
      [zhCN.diagnostics.requested, frames(diagnostic.requestedBufferFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.accepted, frames(diagnostic.acceptedBufferFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.effective, frames(diagnostic.effectiveBufferFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.capacity, frames(diagnostic.capacityFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.startThreshold, frames(diagnostic.startThresholdFrames, zhCN.diagnostics.audioTrackMeaning)],
      [zhCN.diagnostics.preRoll, frames(diagnostic.preRollFrames, zhCN.diagnostics.audioTrackMeaning)],
    ] },
  ] : [];
  const statusText = [
    `${zhCN.diagnostics.state}：${readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}`,
    `${zhCN.diagnostics.serviceMessage}：${snapshot.message}`, `${zhCN.diagnostics.actualConnection}：${active ? actualConnectionLabel(snapshot) : zhCN.diagnostics.notConnected}`,
    `${zhCN.diagnostics.chosenConnection}：${intendedConnectionLabel(snapshot)}`, `${zhCN.diagnostics.receiverBackend}：${snapshot.settings.audioBackend}`,
    `${zhCN.diagnostics.device}：${snapshot.sessionReceiverBinding ? bindingReferenceLabel(snapshot.sessionReceiverBinding) : snapshot.deviceName ?? zhCN.diagnostics.none}`,
    `${zhCN.diagnostics.output}：${snapshot.output === "phone" ? zhCN.diagnostics.phone : zhCN.diagnostics.computer}`, `${zhCN.diagnostics.visibleDevices}：${snapshot.devices.length}`,
    `${zhCN.diagnostics.detail}：${snapshot.statusDetail}`,
    `${zhCN.diagnostics.capabilities}：${capabilities ? zhCN.diagnostics.protocolCompatibility(capabilities.controlMajor, capabilities.managementCompatible, capabilities.audioMajor, capabilities.audioCompatible, capabilities.capabilityBits.toString(16), capabilities.versionCode) : zhCN.diagnostics.unknownCapabilities}`,
    `${zhCN.diagnostics.version}：${snapshot.version}`,
  ].join("\n");
  const copyText = [
    `${zhCN.diagnostics.state}：${readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}`,
    `${zhCN.diagnostics.actualConnection}：${active ? actualConnectionLabel(snapshot) : zhCN.diagnostics.notConnected}`,
    `${zhCN.diagnostics.playbackMode}：${playbackModeLabel(snapshot.settings.playbackMode)}`,
    `${zhCN.diagnostics.output}：${snapshot.output === "phone" ? zhCN.diagnostics.phone : zhCN.diagnostics.computer}`,
    `${zhCN.diagnostics.version}：${snapshot.version}`,
    `${zhCN.diagnostics.capabilitySummary}：${capabilities ? zhCN.diagnostics.protocolCompatibility(capabilities.controlMajor, capabilities.managementCompatible, capabilities.audioMajor, capabilities.audioCompatible, capabilities.capabilityBits.toString(16), capabilities.versionCode) : zhCN.diagnostics.unknownCapabilities}`,
    `${zhCN.diagnostics.playbackDiagnostics}：${playbackContext}`,
    ...groups.flatMap(group => [group.title, ...group.metrics.map(([label, value]) => `${label}：${value}`), ...(group.note ? [group.note] : [])]),
  ].join("\n");
  const metricGroup = (group: typeof groups[number]) => <section className="diagnostic-group" key={group.title}>
    <h3>{group.title}</h3>
    <dl>{group.metrics.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
    {group.note && <p>{group.note}</p>}
  </section>;
  return <Modal title={zhCN.diagnostics.dialogTitle} onClose={onClose} className="diagnostics-dialog">
    <section className="diagnostic-overview" aria-label={zhCN.diagnostics.currentState}>
      <dl><div><dt>{zhCN.diagnostics.state}</dt><dd>{readableStatus(snapshot, snapshot.canDisconnect && snapshot.state === "error")}</dd></div>
        <div><dt>{zhCN.diagnostics.actualConnection}</dt><dd>{active ? actualConnectionLabel(snapshot) : zhCN.diagnostics.notConnected}</dd></div>
        <div><dt>{zhCN.diagnostics.playbackMode}</dt><dd>{playbackModeLabel(snapshot.settings.playbackMode)}</dd></div></dl>
      <p className="diagnostic-context" role="status">{playbackContext}</p>
    </section>
    <details className="diagnostic-details">
      <summary>{zhCN.diagnostics.technicalDetails}</summary>
      <p>{zhCN.diagnostics.runState}</p><pre>{statusText}</pre>
      <section className="diagnostic-playback" aria-label={zhCN.diagnostics.playbackDiagnostics}>
        <h3>{zhCN.diagnostics.playbackDiagnostics}</h3>
        {diagnostic ? groups.map(metricGroup) : <p className="diagnostic-empty">{unavailable}</p>}
      </section>
      <div className="dialog-actions"><button className="dialog-secondary" onClick={() => void navigator.clipboard.writeText(copyText)}><Copy size={15} />{zhCN.diagnostics.copyDiagnostics}</button><button className="dialog-secondary" disabled={exporting} onClick={() => void exportSnapshot()}><Download size={15} />{exporting ? zhCN.diagnostics.exporting : zhCN.diagnostics.exportDiagnostics}</button><button className="dialog-secondary" onClick={() => void invokeApp("open_logs")}>{zhCN.diagnostics.openLogs}<ExternalLink size={14} /></button></div>
      {exportPath && <p className="diagnostic-context" role="status">{zhCN.diagnostics.exportSaved(exportPath)}</p>}
      {exportError && <ErrorDetails summary={zhCN.diagnostics.exportFailed} details={exportError} />}
    </details>
  </Modal>;
}
