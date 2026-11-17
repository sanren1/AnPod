import { describe, it, expect } from '@ohos/hypium';
import { application } from '@kit.AbilityKit';
import { preferences } from '@kit.ArkData';
import { AodSettings, OnceGate, migrateAodSettings } from '../../../main/ets/model/AodSettings';

export default function aodSettingsTest() {
  describe('AodSettings', () => {
    it('android_keys_defaults_and_persisted_selection', 0, async () => {
      const context = await application.createModuleContext(application.getApplicationContext(), 'entry');
      const name = 'aod_settings_test';
      const store = preferences.getPreferencesSync(context, { name: name });
      try {
        store.clearSync();
        const defaults = AodSettings.read(store);
        expect(defaults.enabled).assertFalse();
        expect(defaults.clockWeightLevel).assertEqual(9);
        expect(defaults.waveformWeightLevel).assertEqual(3);
        expect(defaults.showProtocol).assertTrue();
        expect(defaults.showAudioFormat).assertFalse();
        defaults.enabled = true;
        defaults.clockScale = 114;
        defaults.showAudioFormat = true;
        defaults.save(store);
        expect(store.getSync('aod_clock_scale', 0)).assertEqual(114);
        await preferences.removePreferencesFromCache(context, name);
        const restored = AodSettings.read(preferences.getPreferencesSync(context, { name: name }));
        expect(restored.enabled).assertTrue();
        expect(restored.clockScale).assertEqual(114);
        expect(restored.showAudioFormat).assertTrue();
      } finally {
        await preferences.deletePreferences(context, name);
      }
    });
    it('clock_font_keeps_android_values_adds_system_font_and_drops_unknown', 0, async () => {
      const context = await application.createModuleContext(application.getApplicationContext(), 'entry');
      const name = 'aod_font_test';
      const store = preferences.getPreferencesSync(context, { name: name });
      try {
        store.clearSync();
        expect(AodSettings.read(store).clockFont).assertEqual(0);
        for (const value of [0, 1, 2, 3, 4]) {
          store.putSync('aod_clock_font', value);
          expect(AodSettings.read(store).clockFont).assertEqual(value);
        }
        const saved = AodSettings.read(store);
        saved.clockFont = 4;
        saved.save(store);
        expect(store.getSync('aod_clock_font', -1)).assertEqual(4);
        for (const value of [5, -1, 2.5]) {
          store.putSync('aod_clock_font', value);
          expect(AodSettings.read(store).clockFont).assertEqual(0);
        }
        store.putSync('aod_clock_font', 'serif');
        expect(AodSettings.read(store).clockFont).assertEqual(0);
      } finally {
        await preferences.deletePreferences(context, name);
      }
    });
    it('once_gate_runs_step_once', 0, () => {
      const gate = new OnceGate();
      let runs = 0;
      gate.run(() => { runs++; });
      gate.run(() => { runs++; });
      expect(runs).assertEqual(1);
    });
    it('draft_store_moves_into_receiver_store_once', 0, async () => {
      const context = await application.createModuleContext(application.getApplicationContext(), 'entry');
      const legacy = preferences.getPreferencesSync(context, { name: 'aod_legacy_test' });
      const store = preferences.getPreferencesSync(context, { name: 'aod_store_test' });
      try {
        legacy.clearSync();
        store.clearSync();
        expect(migrateAodSettings(legacy, store)).assertFalse();
        legacy.putSync('aod_clock_scale', 113);
        legacy.putSync('aod_show_audio_format', true);
        legacy.putSync('aod_enabled', true);
        legacy.putSync('unrelated', 'x');
        store.putSync('aod_enabled', false);  // already in the receiver store: kept
        store.putSync('wireless', true);
        expect(migrateAodSettings(legacy, store)).assertTrue();
        const moved = AodSettings.read(store);
        expect(moved.clockScale).assertEqual(113);
        expect(moved.showAudioFormat).assertTrue();
        expect(moved.enabled).assertFalse();
        expect(store.getSync('wireless', false)).assertTrue();
        expect(store.hasSync('unrelated')).assertFalse();
        expect(legacy.hasSync('aod_clock_scale')).assertFalse();
        store.putSync('aod_clock_scale', 90);
        expect(migrateAodSettings(legacy, store)).assertFalse();
        expect(store.getSync('aod_clock_scale', 0)).assertEqual(90);
      } finally {
        await preferences.deletePreferences(context, 'aod_legacy_test');
        await preferences.deletePreferences(context, 'aod_store_test');
      }
    });
  });
}
