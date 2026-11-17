import { preferences } from '@kit.ArkData';
import { common } from '@kit.AbilityKit';
import { AodSnapshot } from 'libanpod_core.so';

const STORE = 'receiver';
const LEGACY_STORE = 'anpod_receiver';  // the M3 draft kept AOD settings in their own file
const KEYS = ['aod_enabled', 'aod_clock_scale', 'aod_clock_font', 'aod_clock_weight_level', 'aod_waveform_weight_level',
  'aod_show_protocol', 'aod_show_receive_rate', 'aod_show_buffer', 'aod_show_loss', 'aod_show_audio_format',
  'aod_show_underruns', 'aod_show_output_engine'];

// One-time move of the draft's values into the receiver store; values already in the store win.
export function migrateAodSettings(legacy: preferences.Preferences, store: preferences.Preferences): boolean {
  const moved = KEYS.filter((key: string) => legacy.hasSync(key));
  if (moved.length === 0) return false;
  for (const key of moved) {
    if (!store.hasSync(key)) store.putSync(key, legacy.getSync(key, false));
  }
  store.flushSync();
  legacy.clearSync();
  legacy.flushSync();
  return true;
}

export function aodStore(context: common.Context): preferences.Preferences {
  const store = preferences.getPreferencesSync(context, { name: STORE });
  if (migrateAodSettings(preferences.getPreferencesSync(context, { name: LEGACY_STORE }), store)) {
    preferences.deletePreferences(context, LEGACY_STORE).catch((e: Error) => console.error('AOD prefs: ' + e.message));
  }
  return store;
}

export function loadAodSettings(context: common.Context): AodSettings {
  return AodSettings.read(aodStore(context));
}

@Observed
export class AodSettings {
  enabled: boolean = false;
  clockScale: number = 100;
  clockFont: number = 0;
  clockWeightLevel: number = 9;
  waveformWeightLevel: number = 3;
  showProtocol: boolean = true;
  showReceiveRate: boolean = true;
  showBuffer: boolean = true;
  showLoss: boolean = true;
  showAudioFormat: boolean = false;
  showUnderruns: boolean = false;
  showOutputEngine: boolean = false;

  static read(store: preferences.Preferences): AodSettings {
    const value = new AodSettings();
    value.enabled = store.getSync('aod_enabled', value.enabled) as boolean;
    value.clockScale = store.getSync('aod_clock_scale', value.clockScale) as number;
    value.clockFont = store.getSync('aod_clock_font', value.clockFont) as number;
    value.clockWeightLevel = store.getSync('aod_clock_weight_level', value.clockWeightLevel) as number;
    value.waveformWeightLevel = store.getSync('aod_waveform_weight_level', value.waveformWeightLevel) as number;
    value.showProtocol = store.getSync('aod_show_protocol', value.showProtocol) as boolean;
    value.showReceiveRate = store.getSync('aod_show_receive_rate', value.showReceiveRate) as boolean;
    value.showBuffer = store.getSync('aod_show_buffer', value.showBuffer) as boolean;
    value.showLoss = store.getSync('aod_show_loss', value.showLoss) as boolean;
    value.showAudioFormat = store.getSync('aod_show_audio_format', value.showAudioFormat) as boolean;
    value.showUnderruns = store.getSync('aod_show_underruns', value.showUnderruns) as boolean;
    value.showOutputEngine = store.getSync('aod_show_output_engine', value.showOutputEngine) as boolean;
    return value;
  }

  save(store: preferences.Preferences): void {
    store.putSync('aod_enabled', this.enabled);
    store.putSync('aod_clock_scale', this.clockScale);
    store.putSync('aod_clock_font', this.clockFont);
    store.putSync('aod_clock_weight_level', this.clockWeightLevel);
    store.putSync('aod_waveform_weight_level', this.waveformWeightLevel);
    store.putSync('aod_show_protocol', this.showProtocol);
    store.putSync('aod_show_receive_rate', this.showReceiveRate);
    store.putSync('aod_show_buffer', this.showBuffer);
    store.putSync('aod_show_loss', this.showLoss);
    store.putSync('aod_show_audio_format', this.showAudioFormat);
    store.putSync('aod_show_underruns', this.showUnderruns);
    store.putSync('aod_show_output_engine', this.showOutputEngine);
    store.flushSync();
  }
}

// AOD footer labels (English by design). No snapshot (settings preview, no session): every value is "—".
export function aodMetrics(cfg: AodSettings, s: AodSnapshot | undefined, protocol: string): string[] {
  const items: string[] = [];
  if (cfg.showProtocol) items.push('Protocol ' + protocol);
  const bps = s?.bitsPerSecond;
  if (cfg.showReceiveRate) items.push('Recv ' + (bps === undefined ? '—' :
    bps >= 1000000 ? (bps / 1000000).toFixed(2) + ' Mbps' : (bps / 1000).toFixed(0) + ' kbps'));
  if (cfg.showBuffer) items.push('Buffer ' + (s?.bufferMs === undefined ? '—' : s.bufferMs.toFixed(0) + ' ms'));
  if (cfg.showLoss) items.push('Loss ' + (s?.lossPercent === undefined ? '—' : s.lossPercent.toFixed(1) + '%'));
  if (cfg.showAudioFormat) items.push('Format ' + (s?.sampleRate === undefined || s.channels === undefined ||
    s.encoding === undefined ? '—' : (s.sampleRate / 1000) + ' kHz Float32 ' + s.channels + 'ch'));
  if (cfg.showUnderruns) items.push('Underruns ' + (s?.underruns ?? '—'));
  if (cfg.showOutputEngine) items.push('Output ' + (s?.latencyMode === undefined ? '—' :
    s.latencyMode === 'fast' ? 'OHAudio Fast' : 'OHAudio'));
  return items;
}
