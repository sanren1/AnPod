import { AodSettings } from '../model/AodSettings';
import {
  aodScale, baseClockSize, clockLayout, curve, EDGE_FADE_FRACTION, gapBeforeSeparator, GLOW_STROKE_WIDTH,
  MAX_DRIFT_X, MAX_DRIFT_Y, STROKE_WIDTH, wrapMetrics
} from '../model/AodGeometry';

const WEIGHTS = [300, 400, 450, 500, 550, 600, 700, 775, 850];
const FAMILIES = ['rubik', 'sans', 'serif', 'mono'];

// Written in place by the AOD page every frame; only the waveform canvas reads it, so no page rebuild.
export class AodWave {
  left: number[] = [];
  right: number[] = [];
}

interface View {
  w: number;
  h: number;
  scale: number;
  ox: number;
  oy: number;
}

interface ClockText {
  family: string;
  size: number;
  spacing: number;
  left: number;
  width: number;
  top: number;
}

interface WaveFrame {
  view: View;
  x: number;
  width: number;
  center: number;
  amplitude: number;
  stroke: number;
}

@Component
export struct AodDisplay {
  @ObjectLink @Watch('drawText') settings: AodSettings;
  @Prop @Watch('drawText') clock: string = '';
  @Prop @Watch('drawText') status: string = '';
  @Prop @Watch('drawText') metrics: string[] = [];
  @Prop @Watch('drawText') driftSeconds: number = 0;
  @Prop @Watch('drawWave') frameMs: number = 0;
  @Prop previewWidth: number = 0;  // > 0: settings preview of a previewWidth x previewHeight screen
  @Prop previewHeight: number = 0;
  wave: AodWave = new AodWave();
  private text: CanvasRenderingContext2D = new CanvasRenderingContext2D(new RenderingContextSettings(true));
  private waves: CanvasRenderingContext2D = new CanvasRenderingContext2D(new RenderingContextSettings(true));
  private frame?: WaveFrame;
  @State placed: ClockText = { family: '', size: 0, spacing: 0, left: 0, width: 0, top: 0 };
  private registered: string[] = [];

  // Registered before the canvases are ready, so the first frame already measures the real font.
  aboutToAppear(): void {
    this.register('rubik_black');
    this.fontName();
  }

  private fontName(): string {
    const family = FAMILIES[this.settings.clockFont];
    const weight = WEIGHTS[this.settings.clockWeightLevel - 1];
    let filename = family + '_w' + weight;
    if (family === 'rubik' && weight === 400) filename = 'rubik_regular';
    if (family === 'rubik' && weight === 850) filename = 'rubik_black';
    this.register(filename);
    return filename;
  }

  private register(filename: string): void {
    if (!this.registered.includes(filename)) {
      this.getUIContext().getFont().registerFont({ familyName: filename, familySrc: $rawfile('aod/' + filename + '.ttf') });
      this.registered.push(filename);
    }
  }

  // A "px" canvas font size is physical pixels, while measureText answers in vp.
  private px(vp: number): number {
    return this.getUIContext().vp2px(vp);
  }

  private view(c: CanvasRenderingContext2D): View | undefined {
    if (c.width <= 0 || c.height <= 0) return undefined;
    if (this.previewWidth <= 0) return { w: c.width, h: c.height, scale: 1, ox: 0, oy: 0 };
    const scale = Math.min(c.width / this.previewWidth, c.height / this.previewHeight);
    return {
      w: this.previewWidth, h: this.previewHeight, scale: scale,
      ox: (c.width - this.previewWidth * scale) / 2, oy: (c.height - this.previewHeight * scale) / 2
    };
  }

  // Clock, status and metrics: redrawn on a change or a drift step, never per waveform frame.
  private drawText(): void {
    const c = this.text;
    const v = this.view(c);
    c.clearRect(0, 0, c.width, c.height);
    if (v === undefined) return;
    c.save();
    c.translate(v.ox, v.oy);
    c.scale(v.scale, v.scale);
    const w = v.w;
    const h = v.h;
    if (this.previewWidth > 0) {
      c.fillStyle = '#000000';
      c.fillRect(0, 0, w, h);
    }
    const k = aodScale(w, h);
    const preview = this.previewWidth > 0;
    const dx = preview ? 0 : MAX_DRIFT_X * Math.sin(this.driftSeconds / 120 * 2 * Math.PI);
    const dy = preview ? 0 : MAX_DRIFT_Y * Math.cos(this.driftSeconds / 150 * 2 * Math.PI);

    // Footer: status line, then metrics justified across the row with "|" centered in each gap.
    c.letterSpacing = '0vp';
    c.font = this.px(14 * k) + 'px sans-serif';
    c.textBaseline = 'middle';
    const line = Math.max(8 * k, c.measureText('Ag').height);
    const pad = 8 * k;
    const spacing = 2 * k;
    const available = w - 56 * k;
    const widths = this.metrics.map((s: string) => c.measureText(s).width);
    const bar = c.measureText('|').width;
    const separator = this.metrics.length > 1 ? bar + pad * 2 : 0;
    const rows = wrapMetrics(widths, separator, available);
    const footerHeight = line + rows.length * line + Math.max(0, rows.length - 1) * spacing;

    // Clock: Android AodVisualLayout sizing from the reference and widest-time measurements.
    const base = baseClockSize(w, h);
    const ceiling = base * 1.2;
    this.register('rubik_black');
    c.letterSpacing = '-5vp';
    c.font = this.px(base) + 'px rubik_black';
    const reference = c.measureText('04:44').height;
    const name = this.fontName();
    c.letterSpacing = this.settings.clockFont === 0 ? '-5vp' : '0vp';
    c.font = this.px(ceiling) + 'px ' + name;
    let pairs = 0;
    for (let n = 0; n < 60; n++) pairs = Math.max(pairs, c.measureText(String(n).padStart(2, '0')).width);
    let hours = 0;
    for (let n = 0; n < 24; n++) hours = Math.max(hours, c.measureText(String(n).padStart(2, '0')).width);
    // shortcut: widest hour + ":" + widest minute ignores kerning around the colon; exact would measure all 1440.
    const widest = [hours + c.measureText(':').width + pairs, c.measureText('00:00').height];
    const layout = clockLayout(w, h, footerHeight, reference, widest, this.settings.clockScale);
    const footerTop = layout.footerTop + dy;
    // Canvas fillText ignores registered fonts (measureText honours them), so the clock is a Text on top.
    c.font = this.px(layout.clockSize) + 'px ' + name;
    const clockHeight = c.measureText(this.clock).height;
    const placed: ClockText = {
      family: name, size: layout.clockSize * v.scale, spacing: this.settings.clockFont === 0 ? -5 * v.scale : 0,
      left: v.ox + dx * v.scale, width: w * v.scale, top: v.oy + (layout.clockBottom + dy - clockHeight) * v.scale
    };
    if (JSON.stringify(placed) !== JSON.stringify(this.placed)) this.placed = placed;

    c.letterSpacing = '0vp';
    c.font = this.px(14 * k) + 'px sans-serif';
    c.textBaseline = 'middle';
    c.textAlign = 'left';
    const statusWidth = c.measureText(this.status).width;
    const start = (w - (8 * k + 7 * k + statusWidth)) / 2 + dx;
    c.fillStyle = this.status === 'Receiving' ? '#22D3EE' : this.status === 'Connected' ? '#0891B2' :
      this.status === 'Reconnecting' ? '#F59E0B' : '#4B5563';
    c.beginPath();
    c.arc(start + 4 * k, footerTop + line / 2, 4 * k, 0, Math.PI * 2);
    c.fill();
    c.fillStyle = '#E6E6E6';
    c.fillText(this.status, start + 15 * k, footerTop + line / 2);
    c.fillStyle = '#B3B3B3';
    rows.forEach((row: number[], r: number) => {
      const y = footerTop + line + r * (line + spacing) + line / 2;
      const count = row[1] - row[0];
      let natural = separator * (count - 1);
      for (let i = row[0]; i < row[1]; i++) natural += widths[i];
      const gap = count > 1 ? Math.max(0, available - natural) / (count - 1) : 0;
      let x = 28 * k + dx + (count === 1 ? Math.max(0, available - natural) / 2 : 0);
      for (let i = row[0]; i < row[1]; i++) {
        c.fillText(this.metrics[i], x, y);
        x += widths[i];
        if (i < row[1] - 1) {
          c.fillText('|', x + pad + gapBeforeSeparator(gap), y);
          x += separator + gap;
        }
      }
    });
    c.restore();
    this.frame = {
      view: v, x: w * 0.05 + dx, width: w * 0.9, center: layout.wave.centerY + dy,
      amplitude: layout.wave.maximumAmplitude, stroke: (0.25 + this.settings.waveformWeightLevel * 0.25) * k
    };
    this.drawWave();
  }

  private gradient(c: CanvasRenderingContext2D, width: number, color: string): CanvasGradient {
    const gradient = c.createLinearGradient(0, 0, width, 0);
    gradient.addColorStop(0, 'transparent');
    gradient.addColorStop(EDGE_FADE_FRACTION, color);
    gradient.addColorStop(1 - EDGE_FADE_FRACTION, color);
    gradient.addColorStop(1, 'transparent');
    return gradient;
  }

  // Catmull-Rom samples as one bounded Bezier path (Android smoothWaveformPath).
  private path(c: CanvasRenderingContext2D, offsets: number[], width: number, center: number): void {
    const step = width / (offsets.length - 1);
    const y = (i: number) => center - offsets[Math.max(0, Math.min(offsets.length - 1, i))];
    c.beginPath();
    c.moveTo(0, y(0));
    for (let i = 0; i < offsets.length - 1; i++) {
      const low = Math.min(y(i), y(i + 1));
      const high = Math.max(y(i), y(i + 1));
      const cp1 = Math.max(low, Math.min(high, y(i) + (y(i + 1) - y(i - 1)) / 6));
      const cp2 = Math.max(low, Math.min(high, y(i + 1) - (y(i + 2) - y(i)) / 6));
      c.bezierCurveTo((i + 1 / 3) * step, cp1, (i + 2 / 3) * step, cp2, (i + 1) * step, y(i + 1));
    }
  }

  private drawWave(): void {
    const c = this.waves;
    c.clearRect(0, 0, c.width, c.height);
    const f = this.frame;
    if (f === undefined) return;
    c.save();
    c.translate(f.view.ox, f.view.oy);
    c.scale(f.view.scale, f.view.scale);
    c.translate(f.x, 0);
    const left = this.gradient(c, f.width, '#67E8F9');
    const right = this.gradient(c, f.width, '#22D3EE');
    const silent = (s: number[]) => s.every((x: number) => x === 0);
    if (f.amplitude <= 0 || this.wave.left.length < 2 || (silent(this.wave.left) && silent(this.wave.right))) {
      c.lineWidth = 2 * f.stroke;
      for (const style of [left, right]) {
        c.strokeStyle = style;
        c.beginPath();
        c.moveTo(0, f.center);
        c.lineTo(f.width, f.center);
        c.stroke();
      }
    } else {
      const traces = [curve(this.wave.left, f.amplitude, this.frameMs, false),
        curve(this.wave.right, f.amplitude, this.frameMs, true)];
      const styles = [left, right];
      c.globalAlpha = 0.15;
      c.lineWidth = GLOW_STROKE_WIDTH * f.stroke;
      traces.forEach((t: number[], i: number) => { c.strokeStyle = styles[i]; this.path(c, t, f.width, f.center); c.stroke(); });
      c.globalAlpha = 1;
      c.lineWidth = STROKE_WIDTH * f.stroke;
      traces.forEach((t: number[], i: number) => { c.strokeStyle = styles[i]; this.path(c, t, f.width, f.center); c.stroke(); });
    }
    c.restore();
  }

  build() {
    Stack() {
      Canvas(this.text).width('100%').height('100%')
        .onReady(() => this.drawText())
      Canvas(this.waves).width('100%').height('100%')
        .onReady(() => this.drawWave())
      if (this.placed.size > 0) {
        Text(this.clock)
          .fontFamily(this.placed.family).fontSize(this.placed.size).letterSpacing(this.placed.spacing)
          .fontColor('#F7F7F7').maxLines(1).textAlign(TextAlign.Center).width(this.placed.width)
          .position({ x: this.placed.left, y: this.placed.top })
      }
    }
    .width('100%').height('100%')
    .backgroundColor(this.previewWidth > 0 ? Color.Transparent : Color.Black)
    .onAreaChange(() => this.drawText())
    .accessibilityGroup(true)
    .accessibilityText(this.clock + ' ' + this.status + ' ' + this.metrics.join(', '))
  }
}
