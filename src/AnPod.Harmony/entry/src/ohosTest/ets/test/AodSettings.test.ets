import { describe, it, expect } from '@ohos/hypium';
import { application } from '@kit.AbilityKit';
import { preferences } from '@kit.ArkData';
import { AodSettings } from '../../../main/ets/model/AodSettings';

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
  });
}
