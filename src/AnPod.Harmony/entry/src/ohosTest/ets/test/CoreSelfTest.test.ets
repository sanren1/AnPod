import { describe, it, expect } from '@ohos/hypium';
import anpod from 'libanpod_core.so';

export default function coreSelfTest() {
  describe('CoreSelfTest', () => {
    it('nativeSelfTestsPass', 0, () => {
      const failures: string[] = anpod.selfTest();
      expect(failures.join(',')).assertEqual('');
    });
  });
}
