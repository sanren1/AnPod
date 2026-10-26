import { useState } from "react";
import { strings } from "../strings";
import type { Snapshot } from "../appTypes";
import { Modal } from "./Modal";

type Quality = Snapshot["settings"]["audioQuality"];
type Mode = Snapshot["settings"]["playbackMode"];

export function AudioSettingsDialog({ snapshot, busy, canChangeQuality, canChangeMode, qualityHint, onApply, onClose, returnFocusTarget }: {
  snapshot: Snapshot;
  busy: boolean;
  canChangeQuality: (quality: Quality) => boolean;
  canChangeMode: boolean;
  qualityHint?: string;
  onApply: (quality: Quality, mode: Mode) => Promise<void>;
  onClose: () => void;
  returnFocusTarget?: () => HTMLElement | null;
}) {
  const [quality, setQuality] = useState<Quality>(snapshot.settings.audioQuality);
  const [mode, setMode] = useState<Mode>(snapshot.settings.playbackMode);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const changed = quality !== snapshot.settings.audioQuality || mode !== snapshot.settings.playbackMode;
  const apply = async () => {
    setSaving(true);
    setError("");
    try {
      await onApply(quality, mode);
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  return <Modal title={strings.app.audioSettingsTitle} onClose={onClose} closeDisabled={saving} returnFocusTarget={returnFocusTarget} className="audio-settings-dialog">
    <p className="dialog-footnote">{strings.app.audioSettingsHint}</p>
    <fieldset className="audio-quality-settings">
      <legend>{strings.app.playbackMode}</legend>
      <div className="audio-quality-options">
        {(["Balanced", "LowLatency"] as const).map(value => <label className="audio-quality-card" key={value}>
          <input type="radio" name="draftPlaybackMode" value={value} checked={mode === value} disabled={busy || saving || (snapshot.sessionConnectionTransport ? !canChangeMode : !snapshot.canChangeModes)} onChange={() => setMode(value)} />
          <span><strong>{value === "Balanced" ? strings.app.balanced : strings.app.lowLatency}</strong></span>
        </label>)}
      </div>
    </fieldset>
    <fieldset className="audio-quality-settings">
      <legend>{strings.app.audioQualityTitle}</legend>
      <div className="audio-quality-options">
        {(["standard", "hifi", "hires"] as const).map(value => <label className="audio-quality-card" key={value}>
          <input type="radio" name="draftAudioQuality" value={value} checked={quality === value} disabled={busy || saving || !canChangeQuality(value)} onChange={() => setQuality(value)} />
          <span><strong>{strings.app.audioQualityShortLabels[value]}</strong><small>{strings.app.audioQualityLabels[value]}</small></span>
        </label>)}
      </div>
      {qualityHint && <p className="dialog-footnote">{qualityHint}</p>}
    </fieldset>
    {error && <p className="status-description actionable-error" role="alert">{error}</p>}
    <div className="dialog-actions">
      <button className="dialog-secondary" type="button" disabled={saving} onClick={onClose}>{strings.app.cancelAudioSettings}</button>
      <button className="primary-button" type="button" disabled={busy || saving || !changed} onClick={() => void apply()}>{saving ? strings.app.settingsSavePending : strings.app.applyAudioSettings}</button>
    </div>
  </Modal>;
}
