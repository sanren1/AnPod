import { preferences } from '@kit.ArkData';
import { common } from '@kit.AbilityKit';

export function aodStore(context: common.Context): preferences.Preferences {
  return preferences.getPreferencesSync(context, { name: 'anpod_receiver' });
}

export function loadAodSettings(context: common.Context): AodSettings {
  return AodSettings.read(aodStore(context));
}

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
