import { common } from '@kit.AbilityKit';
import { display } from '@kit.ArkUI';
import { AodDisplay, AodWave } from './AodDisplay';
import { AodSettings, aodMetrics, aodStore, loadAodSettings, MAX_METRICS, metricCount } from '../model/AodSettings';
import { floatingSheet } from '../model/SheetLayout';
import anpod from 'libanpod_core.so';

const CARD_RADIUS = 20;
// Index = stored aod_clock_font value.
const FONT_NAMES: ResourceStr[] = ['Rubik', $r('app.string.aod_sans'), $r('app.string.aod_serif'),
  $r('app.string.aod_mono'), $r('app.string.aod_font_system')];
const FONT_VALUES: number[] = [0, 1, 2, 3, 4];

// Content of the settings sheet; display settings and paired PCs open as sheets of their own.
@Component
export struct SettingsSheet {
  @Prop windowWidth: number = 0;
  openAod: () => void = () => {};
  @State hostIds: string[] = [];
  @State unavailable: boolean = false;
  @State settings: AodSettings = new AodSettings();
  @State optionsOpen: boolean = false;
  @State pcsOpen: boolean = false;
  private readonly previewWave: AodWave = new AodWave();
  // Android previews the real screen in landscape: long side x short side.
  private previewWidth: number = 800;
  private previewHeight: number = 360;

  aboutToAppear(): void {
    this.settings = loadAodSettings(this.getUIContext().getHostContext() as common.Context);
    for (let i = 0; i < 96; i++) {
      this.previewWave.left.push(0.22 + 0.65 * Math.pow(Math.sin(i * 0.23), 2));
    }
    this.previewWave.right = this.previewWave.left;
    const screen = display.getDefaultDisplaySync();
    this.previewWidth = Math.max(screen.width, screen.height) / screen.densityPixels;
    this.previewHeight = Math.min(screen.width, screen.height) / screen.densityPixels;
    this.load();
  }

  load(): void {
    try {
      this.hostIds = anpod.listBindings();
      this.unavailable = false;
    } catch (e) {
      this.unavailable = true;
    }
  }

  save(): void {
    this.settings.save(aodStore(this.getUIContext().getHostContext() as common.Context));
  }

  @Builder
  message(text: Resource) {
    Text(text)
      .fontSize(16)
      .fontColor($r('sys.color.ohos_id_color_text_secondary'))
      .padding(24)
  }

  // Builder rows take getters: a value argument would be a stale copy after the setting changes.
  @Builder
  toggleRow(label: Resource, checked: () => boolean, changed: (value: boolean) => void, limited: boolean = false) {
    Row({ space: 12 }) {
      Text(label).layoutWeight(1)
      Toggle({ type: ToggleType.Switch, isOn: checked() })
        .enabled(!limited || checked() || metricCount(this.settings) < MAX_METRICS)
        .accessibilityText(label)
        .onChange((value: boolean) => { changed(value); this.save(); })
    }.width('100%').constraintSize({ minHeight: 48 })
  }

  @Builder
  sliderRow(label: Resource, value: () => number, min: number, max: number, suffix: string,
    changed: (value: number) => void) {
    Row() {
      Text(label).layoutWeight(1)
      Text(value().toString() + suffix)
    }.width('100%')
    Slider({ value: value(), min: min, max: max, step: 1 })
      .width('100%')
      .accessibilityText(label)
      .onChange((v: number, mode: SliderChangeMode) => {
        changed(Math.round(v));
        if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) this.save();
      })
  }

  // A row that opens a sheet, styled like a system settings entry.
  @Builder
  entryRow(label: Resource, open: () => void) {
    Row() {
      Text(label).fontSize(16).fontColor($r('sys.color.ohos_id_color_text_primary')).layoutWeight(1)
      SymbolGlyph($r('sys.symbol.chevron_forward'))
        .fontSize(18)
        .fontColor([$r('sys.color.ohos_id_color_text_secondary')])
    }
    .width('100%')
    .height(56)
    .padding({ left: 16, right: 16 })
    .borderRadius(CARD_RADIUS)
    .backgroundColor($r('sys.color.comp_background_list_card'))
    .onClick(open)
  }

  // System settings style single choice: one card, a radio per font.
  @Builder
  fontList() {
    Text($r('app.string.aod_clock_font')).fontSize(14)
      .fontColor($r('sys.color.ohos_id_color_text_secondary')).margin({ top: 8 })
    Column() {
      ForEach(FONT_VALUES, (font: number) => {
        Row() {
          Text(FONT_NAMES[font]).layoutWeight(1)
          Radio({ value: String(font), group: 'aodClockFont' })
            .checked(this.settings.clockFont === font)
            .hitTestBehavior(HitTestMode.None)
        }
        .width('100%')
        .height(48)
        .padding({ left: 16, right: 8 })
        .accessibilityGroup(true)
        .accessibilitySelected(this.settings.clockFont === font)
        .onClick(() => {
          if (this.settings.clockFont !== font) {
            this.settings.clockFont = font;
            this.save();
          }
        })
      }, (font: number) => String(font))
    }
    .width('100%')
    .borderRadius(CARD_RADIUS)
    .backgroundColor($r('sys.color.comp_background_list_card'))
  }

  @Builder
  options() {
    Column() {
      AodDisplay({ settings: this.settings, clock: '21:38', status: 'Not connected',
        metrics: aodMetrics(this.settings, undefined, '—'), wave: this.previewWave,
        previewWidth: this.previewWidth, previewHeight: this.previewHeight })
        .width('100%').height('30%').constraintSize({ maxHeight: 140 }).padding({ left: 24, right: 24 })
      Scroll() {
        Column({ space: 8 }) {
          this.sliderRow($r('app.string.aod_clock_scale'), () => this.settings.clockScale, 80, 120, '%',
            (v: number) => { this.settings.clockScale = v; })
          this.fontList()
          this.sliderRow($r('app.string.aod_clock_weight'), () => this.settings.clockWeightLevel, 1, 9, '',
            (v: number) => { this.settings.clockWeightLevel = v; })
          this.sliderRow($r('app.string.aod_wave_weight'), () => this.settings.waveformWeightLevel, 1, 9, '',
            (v: number) => { this.settings.waveformWeightLevel = v; })
          Text(metricCount(this.settings) + '/' + MAX_METRICS).fontSize(14)
            .fontColor($r('sys.color.ohos_id_color_text_secondary')).width('100%').textAlign(TextAlign.End)
          this.toggleRow($r('app.string.aod_protocol'), () => this.settings.showProtocol,
            (v: boolean) => { this.settings.showProtocol = v; }, true)
          this.toggleRow($r('app.string.aod_receive_rate'), () => this.settings.showReceiveRate,
            (v: boolean) => { this.settings.showReceiveRate = v; }, true)
          this.toggleRow($r('app.string.aod_buffer'), () => this.settings.showBuffer,
            (v: boolean) => { this.settings.showBuffer = v; }, true)
          this.toggleRow($r('app.string.aod_loss'), () => this.settings.showLoss,
            (v: boolean) => { this.settings.showLoss = v; }, true)
          this.toggleRow($r('app.string.aod_audio_format'), () => this.settings.showAudioFormat,
            (v: boolean) => { this.settings.showAudioFormat = v; }, true)
          this.toggleRow($r('app.string.aod_underruns'), () => this.settings.showUnderruns,
            (v: boolean) => { this.settings.showUnderruns = v; }, true)
          this.toggleRow($r('app.string.aod_output_engine'), () => this.settings.showOutputEngine,
            (v: boolean) => { this.settings.showOutputEngine = v; }, true)
        }.width('100%').padding({ left: 24, right: 24, top: 8, bottom: 8 }).alignItems(HorizontalAlign.Start)
      }.width('100%').layoutWeight(1).scrollBar(BarState.Off)
      Column() {
        Button($r('app.string.aod_settings_close'))
          .buttonStyle(ButtonStyleMode.NORMAL)
          .width('100%')
          .height(48)
          .onClick(() => { this.optionsOpen = false; })
      }.width('100%').padding({ left: 24, right: 24, top: 8, bottom: 16 })
    }.width('100%').height('100%')
  }

  // Forgetting cannot be undone (the PC must pair again), so it asks first.
  confirmForget(hostId: string): void {
    this.getUIContext().showAlertDialog({
      title: $r('app.string.forget_title'),
      message: $r('app.string.forget_message'),
      autoCancel: true,
      primaryButton: { value: $r('app.string.cancel'), action: () => {} },
      secondaryButton: {
        value: $r('app.string.forget'), fontColor: $r('app.color.status_error'),
        action: () => {
          anpod.forgetBinding(hostId);
          this.load();
        }
      }
    });
  }

  @Builder
  pairedPcs() {
    Scroll() {
      Column() {
        if (this.unavailable) {
          this.message($r('app.string.store_unavailable'))
        } else if (this.hostIds.length === 0) {
          this.message($r('app.string.no_paired_pcs'))
        } else {
          Column() {
            ForEach(this.hostIds, (hostId: string) => {
              Row() {
                Text($r('app.string.pc_label', hostId.slice(0, 4)))
                  .fontSize(18)
                  .fontColor($r('sys.color.ohos_id_color_text_primary'))
                Blank()
                Button($r('app.string.forget'))
                  .buttonStyle(ButtonStyleMode.NORMAL)
                  .fontColor($r('app.color.status_error'))
                  .height(40)
                  .onClick(() => this.confirmForget(hostId))
              }
              .width('100%')
              .height(64)
              .padding({ left: 16, right: 8 })
            }, (hostId: string) => hostId)
          }
          .borderRadius(CARD_RADIUS)
          .backgroundColor($r('sys.color.comp_background_list_card'))
        }
      }.width('100%').padding({ left: 16, right: 16, bottom: 24 })
    }.width('100%').scrollBar(BarState.Off)
  }

  // Scrolls in short windows (landscape phone, split screen); fits its content otherwise.
  build() {
    Scroll() {
      Column({ space: 12 }) {
        Column() {
          Row({ space: 12 }) {
            Text($r('app.string.aod_auto')).fontSize(16).layoutWeight(1)
            Toggle({ type: ToggleType.Switch, isOn: this.settings.enabled })
              .accessibilityText($r('app.string.aod_auto'))
              .onChange((value: boolean) => { this.settings.enabled = value; this.save(); })
          }.width('100%').height(56).padding({ left: 16, right: 16 })
        }
        .borderRadius(CARD_RADIUS)
        .backgroundColor($r('sys.color.comp_background_list_card'))
        Column() {
          this.entryRow($r('app.string.aod_options'), () => { this.optionsOpen = true; })
        }
        .bindSheet($$this.optionsOpen, this.options(),
          floatingSheet($r('app.string.aod_options'), this.windowWidth, SheetSize.LARGE))
        Column() {
          this.entryRow($r('app.string.paired_pcs'), () => { this.load(); this.pcsOpen = true; })
        }
        .bindSheet($$this.pcsOpen, this.pairedPcs(),
          floatingSheet($r('app.string.paired_pcs'), this.windowWidth, SheetSize.FIT_CONTENT))
        Button($r('app.string.aod_open')).width('100%').height(48).margin({ top: 4 })
          .onClick(() => this.openAod())
      }
      .width('100%')
      .padding({ left: 16, right: 16, bottom: 24 })
    }
    .width('100%')
    .scrollBar(BarState.Off)
  }
}
