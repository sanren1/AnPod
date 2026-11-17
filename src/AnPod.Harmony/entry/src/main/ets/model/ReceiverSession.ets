// Single source of UI state for the wired session: native events land in AppStorage keys that the pages read.
import { preferences } from '@kit.ArkData';
import { common } from '@kit.AbilityKit';
import anpod, { CoreEvent } from 'libanpod_core.so';
import { LaunchParams } from './LaunchParams';

export const KEY_STATE = 'coreState';      // 'idle' | CoreState
export const KEY_FORMAT = 'formatText';    // actual opened format, '' when none
export const KEY_CODE = 'pairingCode';     // '' when no pairing dialog should show
export const KEY_PC_CONFIRMED = 'pcConfirmed';
export const KEY_ERROR = 'errorMessage';
export const KEY_HOST = 'activeHost';     // hostIdHex of the wireless peer, '' when none

export interface SessionHooks {
  live: () => void;   // a session started: hold the continuous task and AVSession
  ended: () => void;  // it ended: release them
}

// e.g. "48 kHz | Float32 | 2ch | Fast"; the latency part is left out while the system has not reported it.
export function formatText(e: CoreEvent): string {
  const khz = (e.sampleRate ?? 0) / 1000;
  const latency = e.latencyMode === undefined ? '' : ` | ${e.latencyMode === 'fast' ? 'Fast' : 'Normal'}`;
  return `${khz} kHz | Float32 | ${e.channels ?? 0}ch${latency}`;
}

function set(key: string, value: string | boolean): void {
  AppStorage.setOrCreate(key, value);
}

export class ReceiverSession {
  private static prefs?: preferences.Preferences;
  private static hooks: SessionHooks;
  private static generation: number = 0;

  static initialize(context: common.UIAbilityContext, hooks: SessionHooks): void {
    ReceiverSession.hooks = hooks;
    ReceiverSession.prefs = preferences.getPreferencesSync(context, { name: 'receiver' });
  }

  static restore(): void {
    const wireless = ReceiverSession.prefs?.getSync('wireless', false) === true;
    if (wireless) {
      ReceiverSession.wireless('');
    } else {
      ReceiverSession.reset('wired', 'idle');
    }
  }

  private static reset(mode: string, state: string): number {
    ReceiverSession.generation++;
    set('aodDismissed', false);
    set('transportPage', mode);
    set(KEY_STATE, state);
    set(KEY_FORMAT, '');
    set(KEY_CODE, '');
    set(KEY_PC_CONFIRMED, false);
    set(KEY_ERROR, '');
    set(KEY_HOST, '');
    return ReceiverSession.generation;
  }

  static selectMode(wireless: boolean): void {
    ReceiverSession.disconnect();
    ReceiverSession.hooks.ended();  // the old run's own end event is dropped once the page switches
    const prefs = ReceiverSession.prefs;
    if (prefs !== undefined) {
      prefs.putSync('wireless', wireless);
      prefs.flushSync();
    }
    if (wireless) {
      ReceiverSession.wireless('');
    } else {
      ReceiverSession.reset('wired', 'idle');
    }
  }

  static wireless(hostId: string): void {
    const generation = ReceiverSession.reset('wireless', hostId === '' ? 'listening' : 'searching');
    set(KEY_HOST, hostId);
    try {
      anpod.startWireless(hostId, (e: CoreEvent) => ReceiverSession.deliver(generation, e));
    } catch (e) {
      ReceiverSession.failed(e instanceof Error ? e.message : String(e));
    }
  }

  static start(params: LaunchParams): void {
    const generation = ReceiverSession.reset('wired', 'connecting');
    ReceiverSession.hooks.live();
    try {
      anpod.startHdcSession(params.port, params.hostId, params.launchToken,
        (e: CoreEvent) => ReceiverSession.deliver(generation, e));
    } catch (e) {
      ReceiverSession.failed(e instanceof Error ? e.message : String(e));
    }
  }

  private static deliver(generation: number, e: CoreEvent): void {
    if (generation === ReceiverSession.generation) {
      ReceiverSession.onEvent(e, ReceiverSession.hooks);
    }
  }

  private static failed(message: string): void {
    set(KEY_ERROR, message);
    set(KEY_STATE, 'failed');
    ReceiverSession.hooks.ended();
  }

  static answerPairing(accept: boolean): void {
    set(KEY_CODE, '');
    anpod.answerPairing(accept);
  }

  // USER_STOPPED: the PC drops its reconnect intent. The continuous task is released on the run's end event,
  // after the ACK carrying USER_STOPPED went out.
  static disconnect(): void {
    anpod.stopSession(true);
  }

  // Closing the app: the ACK goes out with the PC's next command (heartbeat every second), so wait for the
  // run's end a little longer than one heartbeat before the process may go away.
  static closeApp(): void {
    anpod.stopSession(true, 1500);
  }

  private static onEvent(e: CoreEvent, hooks: SessionHooks): void {
    if (e.kind === 'state' && e.state !== undefined) {
      set(KEY_STATE, e.state);
      if (e.host !== undefined) {
        set(KEY_HOST, e.host);
      } else if (e.state === 'listening') {
        set('aodDismissed', false);
        set(KEY_HOST, '');
      }
      if (e.state === 'reconnecting') {
        set(KEY_FORMAT, '');
      }
      if (e.state === 'connected' || e.state === 'playing') {
        hooks.live();
      } else if (e.state === 'listening' || e.state === 'searching') {
        set(KEY_FORMAT, '');
        hooks.ended();
      }
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
