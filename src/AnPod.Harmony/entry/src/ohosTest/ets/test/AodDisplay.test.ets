import { describe, expect, it } from '@ohos/hypium';
import { AodGap, curve, wrapMetrics } from '../../../main/ets/model/AodGeometry';

export default function aodDisplayTest() {
  describe('AodDisplay', () => {
    it('aod_grace_recovers_and_idle_preview_stays_open', 0, () => {
      const gap = new AodGap();
      expect(gap.expired(false, 100000)).assertFalse();
      gap.expired(true, 100001);
      gap.expired(false, 100002);
      expect(gap.expired(false, 114999)).assertFalse();
      gap.expired(true, 115000);
      gap.expired(false, 115001);
      expect(gap.expired(false, 130002)).assertTrue();
    });
    it('aod_channels_mirror_and_preserve_silence', 0, () => {
      const history = [0.7, 0.4, 0.3];
      const left = curve(history, 50, 0, false);
      const right = curve(history, 50, 0, true);
      expect(left.length).assertEqual(5);
      expect(left[2] > 0).assertTrue();
      expect(left[2]).assertEqual(-right[2]);
      expect(left[1]).assertEqual(left[3]);
      expect(curve([0, 0], 50, 0, false)[1]).assertEqual(0);
    });
    it('aod_footer_wraps_at_whole_metric_boundaries', 0, () => {
      const rows = wrapMetrics(['A', 'BB', 'CCC'], 6, (s: string) => s.length);
      expect(rows.join('/')).assertEqual('A | BB/CCC');
    });
  });
}
