// Single source of UI state for the wired session: native events land in AppStorage keys that the pages read.
import anpod, { CoreEvent } from 'libanpod_core.so';
import { LaunchParams } from './LaunchParams';

export const KEY_STATE = 'coreState';      // 'idle' | CoreState
export const KEY_FORMAT = 'formatText';    // actual opened format, '' when none
export const KEY_CODE = 'pairingCode';     // '' when no pairing dialog should show
export const KEY_PC_CONFIRMED = 'pcConfirmed';
export const KEY_ERROR = 'errorMessage';

export interface SessionHooks {
  live: () => void;   // a session started: hold the continuous task and AVSession
  ended: () => void;  // it ended: release them
}

// e.g. "48 kHz | Float32 | 2ch | Fast"
export function formatText(e: CoreEvent): string {
  const khz = (e.sampleRate ?? 0) / 1000;
  return `${khz} kHz | Float32 | ${e.channels ?? 0}ch | ${e.latencyMode === 'fast' ? 'Fast' : 'Normal'}`;
}

function set(key: string, value: string | boolean): void {
  AppStorage.setOrCreate(key, value);
}

export class ReceiverSession {
  static start(params: LaunchParams, hooks: SessionHooks): void {
    set(KEY_STATE, 'connecting');
    set(KEY_FORMAT, '');
    set(KEY_CODE, '');
    set(KEY_PC_CONFIRMED, false);
    set(KEY_ERROR, '');
    hooks.live();
    try {
      anpod.startHdcSession(params.port, params.hostId, params.launchToken,
        (e: CoreEvent) => ReceiverSession.onEvent(e, hooks));
    } catch (e) {
      set(KEY_ERROR, e instanceof Error ? e.message : String(e));
      set(KEY_STATE, 'failed');
      hooks.ended();
    }
  }

  static answerPairing(accept: boolean): void {
    set(KEY_CODE, '');
    anpod.answerPairing(accept);
  }

  // USER_STOPPED: the PC drops its reconnect intent.
  static disconnect(): void {
    anpod.stopSession(true);
  }

  private static onEvent(e: CoreEvent, hooks: SessionHooks): void {
    if (e.kind === 'state' && e.state !== undefined) {
      set(KEY_STATE, e.state);
      if (e.state !== 'pairing') {
        set(KEY_CODE, '');
      }
      if (e.state === 'stopped' || e.state === 'failed') {
        set(KEY_FORMAT, '');
        hooks.ended();
      }
    } else if (e.kind === 'pairingCode' && e.code !== undefined) {
      set(KEY_PC_CONFIRMED, e.pcConfirmed === true);
      set(KEY_CODE, e.code);
    } else if (e.kind === 'format') {
      set(KEY_FORMAT, formatText(e));
    } else if (e.kind === 'error' && e.message !== undefined) {
      set(KEY_ERROR, e.message);
    }
  }
}
