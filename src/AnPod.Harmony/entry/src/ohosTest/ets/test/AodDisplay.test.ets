import { describe, expect, it } from '@ohos/hypium';
import {
  AodGap, aodScale, clockFace, clockLayout, curve, fitClockSize, gapBeforeSeparator, VolumeDrag, waveLayout,
  wrapMetrics
} from '../../../main/ets/model/AodGeometry';

function near(actual: number, expected: number): boolean {
  return Math.abs(actual - expected) < 0.01;
}

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
    // AodMetricsGeometryTest vectors.
    it('aod_footer_wraps_whole_metrics_and_centers_separators', 0, () => {
      const rows = wrapMetrics([40, 42, 38, 50], 12, 100);
      expect(JSON.stringify(rows)).assertEqual('[[0,2],[2,4]]');
      expect(JSON.stringify(wrapMetrics([], 12, 100))).assertEqual('[]');
      expect(gapBeforeSeparator(20)).assertEqual(10);
      expect(gapBeforeSeparator(21)).assertEqual(10);
    });
    // PcmWaveformTapTest.waveformLayoutKeepsGlowClearOfClockAndFooter + measuredClockFitHonorsWidthAndHeightBounds.
    it('aod_android_wave_layout_and_clock_fit_vectors', 0, () => {
      const playback = waveLayout(480, 880, 8, 8, 12);
      expect(near(playback.centerY, 680)).assertTrue();
      expect(Math.abs(playback.maximumAmplitude - (184 - 6) * 0.9 * 2 / 3) < 0.05).assertTrue();
      const measure = (s: number) => [Math.round(s * 1.4 + 20), Math.round(s * 1.1 + 12)];
      const fitted = fitClockSize(120, 180, 100, measure);
      expect(fitted < 120).assertTrue();
      expect(measure(fitted)[0] <= 180 && measure(fitted)[1] <= 100).assertTrue();
    });
    // Expected values from a direct port of AodActivity/AodWaveformGeometry/AodDisplaySettings (Python, task-2 report).
    it('aod_phone_360_matches_android', 0, () => {
      const a = clockLayout(800, 360, 40, 160, [520, 192], 100);
      expect(a.k).assertEqual(1);
      expect(a.footerTop).assertEqual(298);
      expect(a.clockBottom).assertEqual(196);
      expect(near(a.clockSize, 128.25)).assertTrue();
      expect(a.wave.centerY).assertEqual(247);
      expect(a.wave.regionHeight).assertEqual(76);
      expect(near(a.wave.maximumAmplitude, 19.2)).assertTrue();
      expect(near(clockLayout(800, 360, 40, 160, [900, 150], 120).clockSize, 135)).assertTrue();
      expect(near(clockLayout(800, 360, 40, 160, [900, 150], 200).clockSize, 135)).assertTrue();
    });
    it('aod_tablet_800_scales_by_two_and_fits_width', 0, () => {
      expect(aodScale(1280, 800)).assertEqual(2);
      const landscape = clockLayout(1280, 800, 80, 373, [1100, 450], 120);
      expect(landscape.k).assertEqual(2);
      expect(landscape.footerTop).assertEqual(800 - 44 - 80);
      expect(landscape.clockSize > 184).assertTrue();
      expect(Math.ceil(1100 * landscape.clockSize / (800 * 0.42 * 1.2)) <= 1280 * 0.92 - 32).assertTrue();
      const portrait = clockLayout(800, 1280, 80, 373, [1100, 450], 120);
      expect(portrait.k).assertEqual(2);
      expect(Math.ceil(1100 * portrait.clockSize / (800 * 0.42 * 1.2)) <= 800 * 0.92 - 32).assertTrue();
      expect(portrait.wave.regionHeight > 0).assertTrue();
    });
    it('aod_scale_caps_at_2_2', 0, () => {
      expect(aodScale(2560, 1600)).assertEqual(2.2);
      expect(aodScale(1600, 2560)).assertEqual(2.2);
      expect(aodScale(390, 844)).assertEqual(1);
      expect(clockLayout(2560, 1600, 80, 600, [1500, 700], 100).k).assertEqual(2.2);
    });
    it('aod_clock_face_maps_fonts_and_weights', 0, () => {
      expect(clockFace(0, 9).family).assertEqual('rubik_black');
      expect(clockFace(0, 9).file).assertEqual('rubik_black');
      expect(clockFace(0, 2).family).assertEqual('rubik_regular');
      expect(clockFace(0, 1).family).assertEqual('rubik_w300');
      expect(clockFace(0, 5).weight).assertEqual(400);  // the file carries the weight
      expect(clockFace(1, 1).family).assertEqual('Noto Sans');
      expect(clockFace(2, 1).family).assertEqual('serif');
      expect(clockFace(3, 1).family).assertEqual('monospace');
      expect(clockFace(4, 1).family).assertEqual('HarmonyOS Sans');
      expect(clockFace(4, 1).file).assertEqual('');
      // System fonts take the nearest weight the text engine accepts (100..900 in steps of 100).
      expect(clockFace(4, 1).weight).assertEqual(300);
      expect(clockFace(4, 3).weight).assertEqual(500);
      expect(clockFace(4, 8).weight).assertEqual(800);
      expect(clockFace(4, 9).weight).assertEqual(900);
      // Unknown stored values fall back to Rubik and the default weight.
      expect(clockFace(5, 9).family).assertEqual('rubik_black');
      expect(clockFace(-1, 9).family).assertEqual('rubik_black');
      expect(clockFace(1.5, 9).family).assertEqual('rubik_black');
      expect(clockFace(4, 0).weight).assertEqual(900);
      expect(clockFace(0, 12).family).assertEqual('rubik_black');
    });
    // AodVolumeDragTest vectors.
    it('aod_volume_drag_keeps_sub_steps_and_clamps', 0, () => {
      const drag = new VolumeDrag(5, 0, 10, 200);
      expect(drag.move(-40)).assertEqual(7);
      expect(drag.move(60)).assertEqual(4);
      const small = new VolumeDrag(5, 0, 10, 200);
      expect(small.move(-4)).assertUndefined();
      expect(small.move(-4)).assertUndefined();
      expect(small.move(-4)).assertEqual(6);
      expect(small.move(-1)).assertUndefined();
      const top = new VolumeDrag(9, 0, 10, 200);
      expect(top.move(-400)).assertEqual(10);
      expect(top.move(-400)).assertUndefined();
      expect(top.move(20)).assertEqual(9);
      const bounded = new VolumeDrag(5, 2, 12, 200);
      expect(bounded.move(-1000)).assertEqual(12);
      expect(bounded.move(20)).assertEqual(11);
      expect(bounded.move(1000)).assertEqual(2);
      expect(bounded.move(-20)).assertEqual(3);
    });
  });
}
