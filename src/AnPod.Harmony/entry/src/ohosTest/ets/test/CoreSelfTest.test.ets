import { describe, it, expect } from '@ohos/hypium';
import anpod, { CoreEvent } from 'libanpod_core.so';
import { formatText } from '../../../main/ets/model/ReceiverSession';

export default function coreSelfTest() {
  describe('CoreSelfTest', () => {
    it('nativeSelfTestsPass', 0, () => {
      const failures: string[] = anpod.selfTest();
      expect(failures.join(',')).assertEqual('');
    });
    it('formatTextKeepsUnknownLatency', 0, () => {
      const known: CoreEvent = { kind: 'format', sampleRate: 48000, channels: 2, encoding: 'float32', latencyMode: 'fast' };
      const unknown: CoreEvent = { kind: 'format', sampleRate: 48000, channels: 2, encoding: 'float32' };
      expect(formatText(known)).assertEqual('48 kHz | Float32 | 2ch | Fast');
      expect(formatText(unknown)).assertEqual('48 kHz | Float32 | 2ch');
    });
  });
}
