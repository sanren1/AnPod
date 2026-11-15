export type CoreState = 'listening' | 'searching' | 'connecting' | 'pairing' | 'connected' | 'playing' | 'paused' |
  'reconnecting' | 'stopped' | 'failed';
// pairingCode is sent when the code shows and again with pcConfirmed: true once the PC user confirmed it.
export interface CoreEvent { kind: 'state' | 'pairingCode' | 'format' | 'error'; state?: CoreState; code?: string;
  sampleRate?: number; channels?: number; encoding?: 'float32'; latencyMode?: 'normal' | 'fast'; message?: string;
  pcConfirmed?: boolean; host?: string; }  // host: hostIdHex of the wireless peer
export function startHdcSession(port: number, hostIdHex: string, launchTokenHex: string, onEvent: (e: CoreEvent) => void): void;
export function answerPairing(accept: boolean): void;
export function stopSession(userInitiated: boolean): void;
export function listBindings(): string[];      // hostIdHex
export function forgetBinding(hostIdHex: string): boolean;
export function selfTest(): string[];
export function selfTestLoopback(port: number): string[];  // for Task 9
export function startWireless(hostIdHex: string, onEvent: (e: CoreEvent) => void): void;
