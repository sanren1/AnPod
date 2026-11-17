// Platform-free AOD geometry. Phones (short side <= 400 vp) follow Android AodWaveformGeometry,
// AodMetricsGeometry and AodDisplaySettings exactly; larger screens scale by k (spec section 16).
export const STROKE_WIDTH = 3.6;
export const GLOW_STROKE_WIDTH = 12;
export const EDGE_FADE_FRACTION = 0.16;
export const MAX_DRIFT_X = 16;
export const MAX_DRIFT_Y = 9;
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
  const bottom = clockBottom(height, referenceHeight, footerTop, MAX_DRIFT_Y, clearance, 64 * k);
  const ceiling = baseClockSize(width, height) * 1.2;
  const maximumWidth = Math.max(0, width * 0.92 - 2 * MAX_DRIFT_X);
  const maximumHeight = Math.max(0, bottom - (16 + MAX_DRIFT_Y));
  const fitted = fitClockSize(ceiling, Math.floor(maximumWidth), Math.floor(maximumHeight),
    (size: number) => [Math.ceil(widest[0] * size / ceiling), Math.ceil(widest[1] * size / ceiling)]);
  return {
    k: k, footerTop: footerTop, clockBottom: bottom, clockSize: scaledClockSize(fitted, clockScale),
    wave: waveLayout(bottom, footerTop, MAX_DRIFT_Y, clearance, GLOW_STROKE_WIDTH * k)
  };
}

// Android AodMetricsGeometry.wrap: whole metrics per row as [start, end) index pairs.
export function wrapMetrics(itemWidths: number[], separatorWidth: number, availableWidth: number): number[][] {
  const rows: number[][] = [];
  let start = 0;
  let rowWidth = 0;
  for (let index = 0; index < itemWidths.length; index++) {
    const added = itemWidths[index] + (index === start ? 0 : separatorWidth);
    if (index > start && rowWidth + added > availableWidth) {
      rows.push([start, index]);
      start = index;
      rowWidth = itemWidths[index];
    } else {
      rowWidth += added;
    }
  }
  if (start < itemWidths.length) rows.push([start, itemWidths.length]);
  return rows;
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
