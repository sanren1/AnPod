import { window } from '@kit.ArkUI';
import { common } from '@kit.AbilityKit';
import { systemDateTime } from '@kit.BasicServicesKit';
import anpod, { AodSnapshot } from 'libanpod_core.so';
import { AodDisplay } from '../components/AodDisplay';
import { AodSettings, loadAodSettings } from '../model/AodSettings';
import { AodGap } from '../model/AodGeometry';

let windowChanges: Promise<void> = Promise.resolve();
function uptime(): number {
  return systemDateTime.getUptime(systemDateTime.TimeType.STARTUP);
}

@Entry
@Component
struct Aod {
  @StorageProp('aodForeground') @Watch('syncVisibility') foreground: boolean = false;
  @StorageProp('coreState') state: string = 'idle';
  @StorageProp('transportPage') transport: string = 'wired';
  @State settings: AodSettings = new AodSettings();
  @State clock: string = '';
  @State elapsedMs: number = 0;
  @State left: number[] = [];
  @State right: number[] = [];
  @State status: string = '';
  @State metrics: string[] = [];
  @State showClose: boolean = true;
  private shown: boolean = false;
  private active: boolean = false;
  private timer: number = -1;
  private lastTouch: number = 0;
  private nextDetails: number = 0;
  private gap: AodGap = new AodGap();
  private savedWindow?: window.Window;
  private savedBrightness: number = -1;
  private savedKeepOn: boolean = false;
  private visibleBars: window.SpecificSystemBar[] = [];

  onPageShow(): void {
    AppStorage.setOrCreate('aodDismissed', true);
    this.settings = loadAodSettings(this.getUIContext().getHostContext() as common.UIAbilityContext);
    this.shown = true;
    this.syncVisibility();
  }

  onPageHide(): void {
    this.shown = false;
    this.syncVisibility();
  }

  aboutToDisappear(): void { this.onPageHide(); }

  onBackPress(): boolean {
    AppStorage.setOrCreate('aodDismissed', true);
    return false;
  }

  private close(): void {
    AppStorage.setOrCreate('aodDismissed', true);
    this.getUIContext().getRouter().back();
  }

  private syncVisibility(): void {
    const visible = this.shown && this.foreground;
    if (visible === this.active) return;
    this.active = visible;
    if (visible) {
      this.lastTouch = uptime();
      this.nextDetails = 0;
      this.timer = setInterval(() => this.tick(), 33);
      this.tick();
    } else {
      clearInterval(this.timer);
      this.timer = -1;
    }
    windowChanges = windowChanges.then(() => this.applyWindow(visible))
      .catch((e: Error) => console.error('AOD window: ' + e.message));
  }

  private async applyWindow(visible: boolean): Promise<void> {
    if (visible) {
      if (!this.active) return;
      const win = await window.getLastWindow(this.getUIContext().getHostContext() as common.UIAbilityContext);
      if (!this.active) return;
      const properties = win.getWindowProperties();
      this.savedWindow = win;
      this.savedBrightness = properties.brightness;
      this.savedKeepOn = properties.isKeepScreenOn;
      this.visibleBars = [];
      if (win.getWindowAvoidArea(window.AvoidAreaType.TYPE_SYSTEM).topRect.height > 0) this.visibleBars.push('status');
      if (win.getWindowAvoidArea(window.AvoidAreaType.TYPE_NAVIGATION_INDICATOR).bottomRect.height > 0) {
        this.visibleBars.push('navigationIndicator');
      }
      await win.setWindowKeepScreenOn(true);
      await win.setWindowBrightness(properties.brightness < 0 ? 0.08 : Math.min(0.08, properties.brightness));
      for (const bar of this.visibleBars) await win.setSpecificSystemBarEnabled(bar, false);
    } else if (this.savedWindow !== undefined) {
      const win = this.savedWindow;
      this.savedWindow = undefined;
      const changes = [win.setWindowBrightness(this.savedBrightness), win.setWindowKeepScreenOn(this.savedKeepOn)];
      for (const bar of this.visibleBars) changes.push(win.setSpecificSystemBarEnabled(bar, true));
      await Promise.all(changes);
    }
  }

  private tick(): void {
    const now = uptime();
    const snapshot = anpod.getAodSnapshot();
    const connected = this.state === 'playing' || this.state === 'paused' || this.state === 'connected';
    if (this.gap.expired(connected || snapshot.receiving, now)) {
      clearInterval(this.timer);
      this.getUIContext().getRouter().back();
      return;
    }
    this.left = snapshot.left;
    this.right = snapshot.right;
    if (now >= this.nextDetails) {
      const date = new Date();
      this.clock = String(date.getHours()).padStart(2, '0') + ':' + String(date.getMinutes()).padStart(2, '0');
      this.status = snapshot.receiving ? 'Receiving' : connected ? 'Connected' :
        this.state === 'reconnecting' ? 'Reconnecting' : 'Not connected';
      const items = this.labels(snapshot, connected);
      if (items.join('|') !== this.metrics.join('|')) this.metrics = items;
      this.showClose = now - this.lastTouch < 6000;
      this.nextDetails = now + 250;
    }
    this.elapsedMs = now;
  }

  private labels(s: AodSnapshot, connected: boolean): string[] {
    const items: string[] = [];
    const cfg = this.settings;
    if (cfg.showProtocol) items.push('Protocol ' + (connected ? (this.transport === 'wired' ? 'hdc' : 'UDP') : '—'));
    const bps = s.bitsPerSecond;
    if (cfg.showReceiveRate) items.push('Recv ' + (bps === undefined ? '—' :
      bps >= 1000000 ? (bps / 1000000).toFixed(2) + ' Mbps' : (bps / 1000).toFixed(0) + ' kbps'));
    if (cfg.showBuffer) items.push('Buffer ' + (s.bufferMs === undefined ? '—' : s.bufferMs.toFixed(0) + ' ms'));
    if (cfg.showLoss) items.push('Loss ' + (s.lossPercent === undefined ? '—' : s.lossPercent.toFixed(1) + '%'));
    if (cfg.showAudioFormat) items.push('Format ' + (s.sampleRate === undefined || s.channels === undefined ||
      s.encoding === undefined ? '—' : (s.sampleRate / 1000) + ' kHz Float32 ' + s.channels + 'ch'));
    if (cfg.showUnderruns) items.push('Underruns ' + (s.underruns ?? '—'));
    if (cfg.showOutputEngine) items.push('Output ' + (s.latencyMode === undefined ? '—' :
      s.latencyMode === 'fast' ? 'OHAudio Fast' : 'OHAudio'));
    return items;
  }

  build() {
    Stack({ alignContent: Alignment.TopEnd }) {
      AodDisplay({ settings: this.settings, clock: this.clock, left: this.left, right: this.right,
        status: this.status, metrics: this.metrics, elapsedMs: this.elapsedMs })
      Button('Close')
        .height(48).backgroundColor(Color.Transparent).fontSize(13).fontColor('#99FFFFFF')
        .margin({ right: 10, top: 6 }).opacity(this.showClose ? 1 : 0)
        .animation({ duration: 700 }).enabled(this.showClose)
        .onClick(() => this.close())
    }
    .width('100%').height('100%').backgroundColor(Color.Black)
    .onTouch(() => { this.lastTouch = uptime(); this.showClose = true; })
  }
}
