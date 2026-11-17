import { window } from '@kit.ArkUI';
import { common } from '@kit.AbilityKit';
import { audio, AVVolumePanel } from '@kit.AudioKit';
import { systemDateTime } from '@kit.BasicServicesKit';
import anpod from 'libanpod_core.so';
import { AodDisplay, AodWave } from '../components/AodDisplay';
import { AodSettings, aodMetrics, loadAodSettings } from '../model/AodSettings';
import { AodGap, aodScale, VolumeDrag } from '../model/AodGeometry';
import { aodProtocol, aodStatus, KEY_STATE } from '../model/ReceiverSession';

let windowChanges: Promise<void> = Promise.resolve();
function uptime(): number {
  return systemDateTime.getUptime(systemDateTime.TimeType.STARTUP);
}

const VOLUME_GESTURE_TRAVEL = 0.60;  // share of the gesture area height spanning the whole media range

@Entry
@Component
struct Aod {
  @StorageProp('aodForeground') @Watch('syncVisibility') foreground: boolean = false;
  @StorageProp(KEY_STATE) state: string = 'idle';
  @StorageProp('transportPage') transport: string = 'wired';
  @State settings: AodSettings = new AodSettings();
  @State clock: string = '';
  @State frameMs: number = 0;
  @State driftSeconds: number = 0;
  @State status: string = '';
  @State metrics: string[] = [];
  @State showClose: boolean = true;
  @State k: number = 1;
  @State volumeLevel: number = -1;  // AVVolumePanel discards values outside the device range
  private readonly wave: AodWave = new AodWave();
  private shown: boolean = false;
  private active: boolean = false;
  private timer: number = -1;
  private lastTouch: number = 0;
  private nextDetails: number = 0;
  private gap: AodGap = new AodGap();
  private drag?: VolumeDrag;
  private dragY: number = 0;
  private lastVolumeMove: number = -1;
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
      .catch((e: Error) => console.error('AOD window: ' + e.message.slice(0, 200)));
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

  // Per frame only the waveform canvas redraws; text state changes at most every 250 ms.
  private tick(): void {
    const now = uptime();
    const snapshot = anpod.getAodSnapshot();
    const status = aodStatus(this.state, snapshot.receiving);
    if (this.gap.expired(status !== 'Not connected', now)) {
      clearInterval(this.timer);
      AppStorage.setOrCreate('aodDismissed', false);  // Index is disconnected now; the next connection may auto-enter again
      this.getUIContext().getRouter().back();
      return;
    }
    this.wave.left = snapshot.left;
    this.wave.right = snapshot.right;
    if (now >= this.nextDetails) {
      const date = new Date();
      this.clock = String(date.getHours()).padStart(2, '0') + ':' + String(date.getMinutes()).padStart(2, '0');
      this.status = status;
      const items = aodMetrics(this.settings, snapshot, aodProtocol(status, this.transport));
      if (items.join('|') !== this.metrics.join('|')) this.metrics = items;
      this.showClose = now - this.lastTouch < 6000;
      this.driftSeconds = Math.floor(now / 1000);  // Android drifts in 1 s steps
      this.nextDetails = now + 250;
    }
    this.frameMs = now;
  }

  private onVolumeTouch(event: TouchEvent, height: number): void {
    const touch = event.touches[0];
    if (event.type === TouchType.Down) {
      this.drag = undefined;
      this.dragY = touch.y;
      try {
        const volumes = audio.getAudioManager().getVolumeManager();
        const usage = audio.StreamUsage.STREAM_USAGE_MUSIC;
        const min = volumes.getMinVolumeByStream(usage);
        const max = volumes.getMaxVolumeByStream(usage);
        // A level just requested may not be applied yet; continue from it rather than the stale system value.
        const start = this.volumeLevel >= 0 && uptime() - this.lastVolumeMove < 1000 ?
          this.volumeLevel : volumes.getVolumeByStream(usage);
        if (max > min) {
          this.drag = new VolumeDrag(start, min, max, height * VOLUME_GESTURE_TRAVEL);
          // The panel acts only on a changed value: a first step back to the last requested level must still apply.
          this.volumeLevel = start;
        }
      } catch (e) {
        console.error('AOD volume: ' + (e instanceof Error ? e.message : String(e)).slice(0, 200));  // no gesture; volume keys still work
      }
    } else if (event.type === TouchType.Move && this.drag !== undefined) {
      const target = this.drag.move(touch.y - this.dragY);
      this.dragY = touch.y;
      if (target !== undefined) {
        this.volumeLevel = target;
        this.lastVolumeMove = uptime();
      }
    } else if (event.type === TouchType.Up || event.type === TouchType.Cancel) {
      this.drag = undefined;
    }
  }

  build() {
    Stack({ alignContent: Alignment.TopEnd }) {
      AodDisplay({ settings: this.settings, clock: this.clock, status: this.status, metrics: this.metrics,
        driftSeconds: this.driftSeconds, frameMs: this.frameMs, wave: this.wave })
      AVVolumePanel({ volumeLevel: this.volumeLevel }).width(1).height(1)
      Column() {
        Column().width('100%').layoutWeight(1)
          .onTouch((event: TouchEvent) => this.onVolumeTouch(event, event.target.area.height as number))
      }
      .width('33.3%').height('100%').padding({ top: 64, right: 16, bottom: 48 })
      Button($r('app.string.aod_close'))
        .height(48 * this.k).backgroundColor(Color.Transparent).fontSize(13 * this.k).fontColor('#55FFFFFF')
        .margin({ right: 10 * this.k, top: 6 * this.k }).opacity(this.showClose ? 1 : 0)
        .animation({ duration: 700 }).enabled(this.showClose)
        .onClick(() => this.close())
    }
    .width('100%').height('100%').backgroundColor(Color.Black)
    .onAreaChange((_: Area, area: Area) => { this.k = aodScale(area.width as number, area.height as number); })
    .onTouch(() => { this.lastTouch = uptime(); this.showClose = true; })
  }
}
