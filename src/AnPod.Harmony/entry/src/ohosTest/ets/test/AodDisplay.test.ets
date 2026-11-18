import { describe, expect, it } from '@ohos/hypium';
import {
  AodGap, aodScale, clockFace, clockLayout, curve, driftOffset, fitClockSize, fitMetricsFont, gapBeforeSeparator,
  interpolatedWindow, newestBucketAt, VolumeDrag, WAVE_POINTS, BUCKET_MS, waveLayout, waveStrokeScale
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
    it('aod_footer_separator_gap_splits_evenly', 0, () => {
      expect(gapBeforeSeparator(20)).assertEqual(10);
      expect(gapBeforeSeparator(21)).assertEqual(10);
    });
    it('aod_metrics_font_fits_one_line_with_floor', 0, () => {
      expect(fitMetricsFont([100, 100], 20, 400, 1)).assertEqual(14);
      expect(fitMetricsFont([], 0, 400, 2)).assertEqual(28);
      expect(fitMetricsFont([200, 200, 200], 20, 320, 1)).assertEqual(10);
      expect(near(fitMetricsFont([100, 100, 100], 20, 280, 1), 14 * 280 / 340)).assertTrue();
      expect(fitMetricsFont([400, 400, 400], 40, 100, 2)).assertEqual(20);
    });
    it('aod_wave_stroke_uses_new_levels_times_sqrt_k', 0, () => {
      const levels = [0.3, 0.6, 1, 1.25, 1.5, 1.75, 2, 2.25, 2.5];
      levels.forEach((scale: number, i: number) => expect(near(waveStrokeScale(i + 1, 1), scale)).assertTrue());
      expect(near(waveStrokeScale(3, 4), 2)).assertTrue();
      expect(near(waveStrokeScale(1, 2.2), 0.3 * Math.sqrt(2.2))).assertTrue();
      expect(waveStrokeScale(0, 1)).assertEqual(1);  // unknown stored level: default
      expect(waveStrokeScale(10, 1)).assertEqual(1);
    });
    it('aod_drift_scales_with_k_and_keeps_periods', 0, () => {
      expect(JSON.stringify(driftOffset(0, 1))).assertEqual('[0,9]');
      expect(near(driftOffset(30, 2)[0], 32)).assertTrue();  // quarter of the 120 s period
      expect(near(driftOffset(75, 2)[1], -18)).assertTrue();  // half of the 150 s period
      expect(near(driftOffset(120, 2.2)[0], 0)).assertTrue();
      expect(near(driftOffset(150, 2.2)[1], 9 * 2.2)).assertTrue();
      for (let t = 0; t < 600; t += 7) {
        expect(Math.abs(driftOffset(t, 2.2)[0]) <= 16 * 2.2 + 1e-9).assertTrue();
        expect(Math.abs(driftOffset(t, 2.2)[1]) <= 9 * 2.2 + 1e-9).assertTrue();
      }
    });
    it('aod_window_interpolates_between_buckets', 0, () => {
      const points = [0.8, 0.4, 0.2, 0];
      expect(JSON.stringify(interpolatedWindow(points, 1000, 1000))).assertEqual(JSON.stringify(points));
      const half = interpolatedWindow(points, 1005, 1000);
      expect(half[0]).assertEqual(0.8);
      expect(near(half[1], 0.6)).assertTrue();
      expect(near(half[2], 0.3)).assertTrue();
      expect(near(half[3], 0.1)).assertTrue();
      const full = interpolatedWindow(points, 1010, 1000);  // equals the next bucket's window
      expect(JSON.stringify(full)).assertEqual(JSON.stringify([0.8, 0.8, 0.4, 0.2]));
      expect(JSON.stringify(interpolatedWindow(points, 1500, 1000))).assertEqual(JSON.stringify(full));  // clamped
      expect(JSON.stringify(interpolatedWindow(points, 900, 1000))).assertEqual(JSON.stringify(points));  // clock skew
      expect(interpolatedWindow([], 5, 0).length).assertEqual(0);
    });
    it('aod_newest_bucket_time_comes_from_native_age', 0, () => {
      expect(newestBucketAt(5000, 7000000)).assertEqual(4993);  // 7 ms old
      expect(newestBucketAt(5000, 0)).assertEqual(5000);
      expect(newestBucketAt(5000, undefined)).assertEqual(5000);  // no live wave
      expect(newestBucketAt(5000, -3000000)).assertEqual(5000);  // never in the future
      const points = [0.8, 0.4, 0.2, 0];
      const at = newestBucketAt(1000, 5000000);  // 5 ms old = half of a 10 ms bucket
      expect(near(interpolatedWindow(points, 1000, at)[1], 0.6)).assertTrue();
    });
    it('aod_waveform_b_window_ripples_and_phase_speed', 0, () => {
      expect(WAVE_POINTS).assertEqual(120);
      expect(WAVE_POINTS * BUCKET_MS).assertEqual(1200);
      const history: number[] = [];
      for (let i = 0; i < WAVE_POINTS; i++) history.push(1);
      const trace = curve(history, 100, 0, false);
      expect(trace.length).assertEqual(2 * WAVE_POINTS - 1);
      // Phase advance across half the width, in cycles; Android: 48 points, 0.34 rad per point, 15 rad/s.
      const cycles = (age: number, step: number): number => (Math.sqrt(age * age + 16) - 4) * step / (2 * Math.PI);
      const android = 2 * cycles(47, 0.34);
      const here = 2 * cycles(WAVE_POINTS - 1, 0.34 * 48 / WAVE_POINTS);
      expect(near(android, 4.68)).assertTrue();
      expect(Math.abs(here - android) < 0.5).assertTrue();  // about 5 ripples across, like Android
      // Zero crossings of the unmodulated carrier (taper keeps the sign): 2 per cycle.
      let crossings = 0;
      for (let i = 1; i < trace.length; i++) if (trace[i - 1] * trace[i] < 0) crossings++;
      expect(Math.abs(crossings / 2 - here) <= 1).assertTrue();
      // Phase speed 7.5 rad/s: the centre point peaks at 0, is at -1 half a period later, 0 after a full one.
      const centre = (ms: number): number => curve(history, 100, ms, false)[WAVE_POINTS - 1];
      expect(near(centre(0), 100)).assertTrue();
      expect(near(centre(Math.PI / 0.0075), -100)).assertTrue();
      expect(near(centre(2 * Math.PI / 0.0075), 100)).assertTrue();
      expect(near(centre(Math.PI / 2 / 0.0075), 0)).assertTrue();
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
      expect(Math.ceil(1100 * landscape.clockSize / (800 * 0.42 * 1.2)) <= 1280 * 0.92 - 64).assertTrue();
      const portrait = clockLayout(800, 1280, 80, 373, [1100, 450], 120);
      expect(portrait.k).assertEqual(2);
      expect(Math.ceil(1100 * portrait.clockSize / (800 * 0.42 * 1.2)) <= 800 * 0.92 - 64).assertTrue();
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
