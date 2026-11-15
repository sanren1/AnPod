import { describe, it, expect } from '@ohos/hypium';
import { abilityDelegatorRegistry } from '@kit.TestKit';
import anpod from 'libanpod_core.so';

// Driven by src-tauri hdc_record_stream_harmony_emulator_loopback: `aa test ... -s port <tablet port>`.
export default function hdcLoopbackTest() {
  describe('HdcLoopback', () => {
    it('hdc_loopback_session', 0, () => {
      const port = Number(abilityDelegatorRegistry.getArguments().parameters['-s port']);
      expect(Number.isInteger(port) && port > 0).assertTrue();
      expect(anpod.selfTestLoopback(port).join(',')).assertEqual('');
    });
  });
}
