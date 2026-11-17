import { AodSettings } from '../model/AodSettings';
import { curve, wrapMetrics } from '../model/AodGeometry';

const WEIGHTS = [300, 400, 450, 500, 550, 600, 700, 775, 850];
const FAMILIES = ['rubik', 'sans', 'serif', 'mono'];

@Component
export struct AodDisplay {
  @Prop @Watch('draw') settings: AodSettings = new AodSettings();
  @Prop @Watch('draw') clock: string = '';
  @Prop left: number[] = [];
  @Prop right: number[] = [];
  @Prop @Watch('draw') elapsedMs: number = 0;
  @Prop @Watch('draw') status: string = '';
  @Prop @Watch('draw') metrics: string[] = [];
  @Prop preview: boolean = false;
  private ctx: CanvasRenderingContext2D = new CanvasRenderingContext2D(new RenderingContextSettings(true));
  private ready: boolean = false;
  private registered: string[] = [];

  private fontName(): string {
    const family = FAMILIES[this.settings.clockFont];
    const weight = WEIGHTS[this.settings.clockWeightLevel - 1];
    let filename = family + '_w' + weight;
    if (family === 'rubik' && weight === 400) filename = 'rubik_regular';
    if (family === 'rubik' && weight === 850) filename = 'rubik_black';
    if (!this.registered.includes(filename)) {
      this.getUIContext().getFont().registerFont({ familyName: filename, familySrc: $rawfile('aod/' + filename + '.ttf') });
      this.registered.push(filename);
    }
    return filename;
  }

  private trace(history: number[], amplitude: number, right: boolean, width: number, center: number): void {
    const c = this.ctx;
    const offsets = curve(history, amplitude, this.elapsedMs, right);
    const gradient = c.createLinearGradient(0, 0, width, 0);
    const color = right ? '#22D3EE' : '#67E8F9';
    gradient.addColorStop(0, 'transparent');
    gradient.addColorStop(0.16, color);
    gradient.addColorStop(0.84, color);
    gradient.addColorStop(1, 'transparent');
    c.strokeStyle = gradient;
    const stroke = 0.25 + this.settings.waveformWeightLevel * 0.25;
    c.beginPath();
    if (history.every((x: number) => x === 0) || offsets.length < 2) {
      c.moveTo(0, center);
      c.lineTo(width, center);
      c.lineWidth = 2 * stroke;
      c.stroke();
      return;
    }
    const step = width / (offsets.length - 1);
    c.moveTo(0, center - offsets[0]);
    for (let i = 0; i < offsets.length - 1; i++) {
      const a = offsets[i];
      const b = offsets[i + 1];
      const low = Math.min(a, b);
      const high = Math.max(a, b);
      const cp1 = Math.max(low, Math.min(high, a + (b - offsets[Math.max(0, i - 1)]) / 6));
      const cp2 = Math.max(low, Math.min(high, b - (offsets[Math.min(offsets.length - 1, i + 2)] - a) / 6));
      c.bezierCurveTo((i + 1 / 3) * step, center - cp1, (i + 2 / 3) * step, center - cp2,
        (i + 1) * step, center - b);
    }
    c.globalAlpha = 0.15;
    c.lineWidth = 12 * stroke;
    c.stroke();
    c.globalAlpha = 1;
    c.lineWidth = 3.6 * stroke;
    c.stroke();
  }

  private draw(): void {
    if (!this.ready) return;
    const c = this.ctx;
    const scale = this.preview ? Math.min(c.width / 800, c.height / 360) : 1;
    if (scale <= 0) return;
    const w = this.preview ? 800 : c.width;
    const h = this.preview ? 360 : c.height;
    c.clearRect(0, 0, c.width, c.height);
    c.save();
    c.translate((c.width - w * scale) / 2, (c.height - h * scale) / 2);
    c.scale(scale, scale);
    if (!this.preview) {
      c.translate(16 * Math.sin(this.elapsedMs * Math.PI / 60000), 9 * Math.cos(this.elapsedMs * Math.PI / 75000));
    }
    c.textAlign = 'center';
    c.textBaseline = 'middle';
    c.font = '14px sans-serif';
    const rows = wrapMetrics(this.metrics, w - 56, (s: string) => c.measureText(s).width);
    const footer = h - 22 - 19 * (rows.length + 1);
    const lineWidth = c.measureText(this.status).width;
    c.fillStyle = this.status === 'Receiving' ? '#22D3EE' : this.status === 'Connected' ? '#0891B2' :
      this.status === 'Reconnecting' ? '#F59E0B' : '#4B5563';
    c.beginPath();
    c.arc(w / 2 - (lineWidth + 15) / 2 + 4, footer + 9, 4, 0, Math.PI * 2);
    c.fill();
    c.fillStyle = '#E6E6E6';
    c.fillText(this.status, (w + 15) / 2, footer + 9);
    c.fillStyle = '#B3B3B3';
    rows.forEach((s: string, i: number) => c.fillText(s, w / 2, footer + 28 + i * 19));
    const base = Math.max(96, Math.min(184, h * 0.40));
    const bottom = Math.min(h * 0.1 + base * 1.2, footer - 26 - Math.min(64, h * 0.32));
    const name = this.fontName();
    c.font = '100px ' + name;
    let digit = 0;
    for (let i = 0; i < 10; i++) digit = Math.max(digit, c.measureText(String(i)).width);
    const tracking = this.settings.clockFont === 0 ? -5 : 0;
    const widest = 4 * digit + c.measureText(':').width;
    const size = Math.max(0, Math.min(base * 1.2, (w * 0.92 - 32 - 4 * tracking) * 100 / widest,
      (bottom - 25) / 1.2)) * this.settings.clockScale / 120;
    c.font = size + 'px ' + name;
    c.fillStyle = '#F7F7F7';
    c.textAlign = 'left';
    const chars = this.clock.split('');
    const widths = chars.map((ch: string) => c.measureText(ch).width);
    let x = (w - widths.reduce((a: number, b: number) => a + b, 0) - tracking * (chars.length - 1)) / 2;
    chars.forEach((ch: string, i: number) => {
      c.fillText(ch, x, bottom - size * 0.6);
      x += widths[i] + tracking;
    });
    c.translate(w * 0.05, 0);
    const amplitude = Math.max(0, (footer - bottom) / 2 - 19) * 0.60;
    this.trace(this.left, amplitude, false, w * 0.9, (footer + bottom) / 2);
    this.trace(this.right, amplitude, true, w * 0.9, (footer + bottom) / 2);
    c.restore();
  }

  build() {
    Canvas(this.ctx)
      .width('100%').height('100%').backgroundColor(Color.Black)
      .accessibilityText(this.clock + ' ' + this.status + ' ' + this.metrics.join(', '))
      .onReady(() => { this.ready = true; this.draw(); })
  }
}
