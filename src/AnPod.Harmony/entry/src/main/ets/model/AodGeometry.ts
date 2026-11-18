// Platform-free AOD geometry. Phones (short side <= 400 vp) follow Android AodWaveformGeometry,
// AodMetricsGeometry and AodDisplaySettings exactly; larger screens scale by k (spec section 16).
export const STROKE_WIDTH = 3.6;
export const GLOW_STROKE_WIDTH = 12;
export const EDGE_FADE_FRACTION = 0.16;
const DRIFT_X = 16;  // vp at k = 1; the whole display layer drifts by this x k
const DRIFT_Y = 9;
export const BUCKET_MS = 25;
// Section 17: stroke = level scale x sqrt(k); level 3 = 1, stored levels 1..9 unchanged.
const WAVE_LEVEL_SCALES = [0.3, 0.6, 1, 1.25, 1.5, 1.75, 2, 2.25, 2.5];
const AMPLITUDE_FILL = 0.60;
const CARRIER_STEP_RADIANS = 0.34;
const CARRIER_RADIANS_PER_MS = 0.015;
const AMPLITUDE_LOG_RANGE = Math.log1p(10000);

// Android SESSION_GAP_EXIT_MS: a session that went away for this long closes the AOD.
export class AodGap {
  private seen: boolean = false;
  private since: number = -1;

  expired(connected: boolean, now: number): boolean {
    if (connected) {
      this.seen = true;
      this.since = -1;
    } else if (this.seen && this.since < 0) {
      this.since = now;
    }
    return this.since >= 0 && now - this.since > 15000;
  }
}

export function waveStrokeScale(level: number, k: number): number {
  const index = Number.isInteger(level) && level >= 1 && level <= 9 ? level : 3;
  return WAVE_LEVEL_SCALES[index - 1] * Math.sqrt(k);
}

// Burn-in drift [dx, dy] for the whole layer; seconds is the 1 s step counter (Android drifts in 1 s steps).
export function driftOffset(seconds: number, k: number): number[] {
  return [DRIFT_X * k * Math.sin(seconds / 120 * 2 * Math.PI), DRIFT_Y * k * Math.cos(seconds / 150 * 2 * Math.PI)];
}

// Footer metrics stay on one line: font 14k, shrunk in proportion to fit, never below 10k.
// widths and separator are measured at 14k.
export function fitMetricsFont(widths: number[], separator: number, available: number, k: number): number {
  const base = 14 * k;
  const natural = widths.reduce((sum: number, w: number) => sum + w, 0) + separator * Math.max(0, widths.length - 1);
  return natural <= available ? base : Math.max(10 * k, base * available / natural);
}

// history[0] is the newest bucket. The window slides outward by the share of a bucket that has elapsed
// since the newest one arrived, so the trace moves continuously between buckets.
export function interpolatedWindow(points: number[], now: number, newestAt: number): number[] {
  const share = Math.max(0, Math.min(1, (now - newestAt) / BUCKET_MS));
  return points.map((p: number, i: number) => i === 0 ? p : points[i - 1] * share + p * (1 - share));
}

export function aodScale(width: number, height: number): number {
  return Math.min(2.2, Math.max(1, Math.min(width, height) / 400));
}

// Android: clamp(height * 0.40, 96, 184). Above phone size the 184 cap goes and S * 0.42 applies.
export function baseClockSize(width: number, height: number): number {
  const shortSide = Math.min(width, height);
  return shortSide <= 400 ? Math.max(96, Math.min(184, height * 0.40)) : shortSide * 0.42;
}

export function clockBottom(screenHeight: number, baseClockHeight: number, footerTop: number,
  maximumDrift: number, clearance: number, waveformReserveCap: number): number {
  const waveformReserve = Math.min(screenHeight * 0.32, waveformReserveCap);
  return Math.min(screenHeight * 0.10 + baseClockHeight,
    footerTop - 2 * (maximumDrift + clearance) - waveformReserve);
}

export interface WaveLayout {
  centerY: number;
  regionHeight: number;
  maximumAmplitude: number;
}

export function waveLayout(bottom: number, footerTop: number, maximumDrift: number, clearance: number,
  glowStroke: number): WaveLayout {
  const center = (bottom + footerTop) / 2;
  const half = Math.max(0, Math.min(center - bottom, footerTop - center) - maximumDrift - clearance);
  return { centerY: center, regionHeight: half * 2, maximumAmplitude: Math.max(0, half - glowStroke / 2) * AMPLITUDE_FILL };
}

// Android AodDisplaySettings.fitClockSizeSp: measure returns [width, height] at a font size.
export function fitClockSize(requested: number, availableWidth: number, availableHeight: number,
  measure: (size: number) => number[]): number {
  const fits = (size: number): boolean => {
    const m = measure(size);
    return m[0] <= availableWidth && m[1] <= availableHeight;
  };
  if (fits(requested)) return requested;
  let lower = 0;
  let upper = requested;
  for (let i = 0; i < 10; i++) {
    const candidate = (lower + upper) / 2;
    if (fits(candidate)) {
      lower = candidate;
    } else {
      upper = candidate;
    }
  }
  return lower;
}

export function scaledClockSize(maximumSize: number, clockScale: number): number {
  return maximumSize * Math.max(80, Math.min(120, clockScale)) / 120;
}

export interface ClockLayout {
  k: number;
  footerTop: number;
  clockBottom: number;
  clockSize: number;  // after the user's clock scale
  wave: WaveLayout;
}

// widest: [width, height] of the widest "HH:MM" measured at baseClockSize * 1.2; referenceHeight: line
// height of Rubik Black "04:44" at baseClockSize. footerHeight is the status line plus metric rows.
export function clockLayout(width: number, height: number, footerHeight: number, referenceHeight: number,
  widest: number[], clockScale: number): ClockLayout {
  const k = aodScale(width, height);
  const clearance = 4;
  const footerTop = height - 22 * k - footerHeight;
  const driftX = DRIFT_X * k;
  const driftY = DRIFT_Y * k;
  const bottom = clockBottom(height, referenceHeight, footerTop, driftY, clearance, 64 * k);
  const ceiling = baseClockSize(width, height) * 1.2;
  const maximumWidth = Math.max(0, width * 0.92 - 2 * driftX);
  const maximumHeight = Math.max(0, bottom - (16 + driftY));
  const fitted = fitClockSize(ceiling, Math.floor(maximumWidth), Math.floor(maximumHeight),
    (size: number) => [Math.ceil(widest[0] * size / ceiling), Math.ceil(widest[1] * size / ceiling)]);
  return {
    k: k, footerTop: footerTop, clockBottom: bottom, clockSize: scaledClockSize(fitted, clockScale),
    wave: waveLayout(bottom, footerTop, driftY, clearance, GLOW_STROKE_WIDTH * Math.sqrt(k))
  };
}

export function gapBeforeSeparator(distributedGap: number): number {
  return Math.floor(distributedGap / 2);
}

// Android AodWaveformGeometry.centeredCurve: newest bucket at center, older history toward both edges.
export function curve(history: number[], amplitude: number, ms: number, right: boolean): number[] {
  const center = history.length - 1;
  const values: number[] = [];
  if (center < 1) return values;
  for (let i = -center; i <= center; i++) {
    const age = Math.abs(i);
    const level = Math.log1p(Math.max(0, Math.min(1, history[age])) * 10000) / AMPLITUDE_LOG_RANGE;
    const taper = (1 + Math.cos(Math.PI * age / center)) / 2;
    const phase = (Math.sqrt(age * age + 16) - 4) * CARRIER_STEP_RADIANS - ms * CARRIER_RADIANS_PER_MS;
    const value = level * taper * taper * Math.cos(phase) * amplitude;
    values.push(right ? -value : value);
  }
  return values;
}

// Android AodVolumeDrag: keeps sub-step movement and drops overscroll at the volume limits.
export class VolumeDrag {
  private level: number;
  private requested: number;
  private readonly minimum: number;
  private readonly maximum: number;
  private readonly height: number;

  constructor(initial: number, minimum: number, maximum: number, height: number) {
    this.level = initial;
    this.requested = initial;
    this.minimum = minimum;
    this.maximum = maximum;
    this.height = height;
  }

  move(deltaY: number): number | undefined {
    this.level = Math.max(this.minimum, Math.min(this.maximum,
      this.level - deltaY / this.height * (this.maximum - this.minimum)));
    const target = Math.round(this.level);
    if (target === this.requested) return undefined;
    this.requested = target;
    return target;
  }
}

// Android AodDisplaySettings clock weights, one per level 1..9.
const CLOCK_WEIGHTS = [300, 400, 450, 500, 550, 600, 700, 775, 850];
// Values 1..3 keep Android's meaning on system families (fontconfig on API 26); 4 is the system font itself.
const SYSTEM_FAMILIES = ['', 'Noto Sans', 'serif', 'monospace', 'HarmonyOS Sans'];

export interface ClockFace {
  family: string;
  weight: number;  // font weight to request; a bundled Rubik file already carries its weight
  file: string;  // rawfile aod/<file>.ttf to register, '' for a system family
}

// Unknown stored values fall back to Rubik and the default weight level 9.
export function clockFace(font: number, weightLevel: number): ClockFace {
  const level = Number.isInteger(weightLevel) && weightLevel >= 1 && weightLevel <= 9 ? weightLevel : 9;
  const weight = CLOCK_WEIGHTS[level - 1];
  if (Number.isInteger(font) && font >= 1 && font < SYSTEM_FAMILIES.length) {
    return { family: SYSTEM_FAMILIES[font], weight: Math.min(900, Math.round(weight / 100) * 100), file: '' };
  }
  const file = weight === 400 ? 'rubik_regular' : weight === 850 ? 'rubik_black' : 'rubik_w' + weight;
  return { family: file, weight: 400, file: file };
}
