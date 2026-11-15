import { describe, it, expect } from '@ohos/hypium';
import { abilityDelegatorRegistry } from '@kit.TestKit';
import anpod from 'libanpod_core.so';

// Driven by src-tauri hdc_record_stream_harmony_emulator_loopback: `aa test ... -s port <tablet port>`.
export default function hdcLoopbackTest() {
  describe('HdcLoopback', () => {
    it('hdc_loopback_session', 0, () => {
      // hypium internals expose `aa test -s port N` as parameters['-s port'] (verified on the emulator).
      const raw = abilityDelegatorRegistry.getArguments().parameters['-s port'];
      if (raw === undefined) {
        return;  // driven only by the PC e2e test; a plain full run has no port
      }
      const port = Number(raw);
      expect(Number.isInteger(port) && port > 0).assertTrue();
      expect(anpod.selfTestLoopback(port).join(',')).assertEqual('');
    });
  });
}
