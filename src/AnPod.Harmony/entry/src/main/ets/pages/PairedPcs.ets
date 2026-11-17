import { common } from '@kit.AbilityKit';
import { AodDisplay } from '../components/AodDisplay';
import { AodSettings, aodStore, loadAodSettings } from '../model/AodSettings';
import anpod from 'libanpod_core.so';

@Entry
@Component
struct PairedPcs {
  @State hostIds: string[] = [];
  @State unavailable: boolean = false;

  @State settings: AodSettings = new AodSettings();
  @State optionsOpen: boolean = false;
  private readonly previewWave: number[] = [];

  aboutToAppear(): void {
    this.settings = loadAodSettings(this.getUIContext().getHostContext() as common.Context);
    for (let i = 0; i < 96; i++) {
      this.previewWave.push(0.22 + 0.65 * Math.pow(Math.sin(i * 0.23), 2));
    }
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

  @Builder
  message(text: Resource) {
    Text(text)
      .fontSize(16)
      .fontColor($r('sys.color.ohos_id_color_text_secondary'))
      .padding(24)
  }

  save(): void {
    this.settings.save(aodStore(this.getUIContext().getHostContext() as common.Context));
  }

  previewMetrics(): string[] {
    const metrics: string[] = [];
    if (this.settings.showProtocol) metrics.push('USB');
    if (this.settings.showReceiveRate) metrics.push('1.54 Mbps');
    if (this.settings.showBuffer) metrics.push('BUF 48 ms');
    if (this.settings.showLoss) metrics.push('LOSS 0');
    if (this.settings.showAudioFormat) metrics.push('48 kHz / 16-bit');
    if (this.settings.showUnderruns) metrics.push('XRUN 0');
    if (this.settings.showOutputEngine) metrics.push('OHAudio');
    return metrics;
  }

  @Builder
  toggleRow(label: Resource, checked: boolean, changed: (value: boolean) => void) {
    Row({ space: 12 }) {
      Text(label).layoutWeight(1)
      Toggle({ type: ToggleType.Switch, isOn: checked })
        .accessibilityText(label)
        .onChange((value: boolean) => { changed(value); this.save(); })
    }.width('100%').constraintSize({ minHeight: 48 })
  }

  @Builder
  sliderRow(label: Resource, value: number, min: number, max: number, suffix: string,
    changed: (value: number) => void) {
    Row() {
      Text(label).layoutWeight(1)
      Text(value.toString() + suffix)
    }.width('100%')
    Slider({ value: value, min: min, max: max, step: 1 })
      .width('100%')
      .accessibilityText(label)
      .onChange((v: number, mode: SliderChangeMode) => {
        changed(Math.round(v));
        if (mode === SliderChangeMode.End || mode === SliderChangeMode.Click) this.save();
      })
  }

  @Builder
  options() {
    Scroll() {
      Column({ space: 8 }) {
        AodDisplay({ settings: this.settings, clock: '21:38', left: this.previewWave, right: this.previewWave,
          elapsedMs: 0, status: 'Receiving', metrics: this.previewMetrics(), preview: true })
          .width('100%').height(140)
        this.sliderRow($r('app.string.aod_clock_scale'), this.settings.clockScale, 80, 120, '%',
          (v: number) => { this.settings.clockScale = v; })
        Row({ space: 8 }) {
          Text($r('app.string.aod_clock_font')).layoutWeight(1)
          Select([{ value: 'Rubik' }, { value: $r('app.string.aod_sans') },
            { value: $r('app.string.aod_serif') }, { value: $r('app.string.aod_mono') }])
            .selected(this.settings.clockFont)
            .value(this.fontName())
            .accessibilityText($r('app.string.aod_clock_font'))
            .onSelect((index: number) => { this.settings.clockFont = index; this.save(); })
        }.width('100%').constraintSize({ minHeight: 48 })
        this.sliderRow($r('app.string.aod_clock_weight'), this.settings.clockWeightLevel, 1, 9, '',
          (v: number) => { this.settings.clockWeightLevel = v; })
        this.sliderRow($r('app.string.aod_wave_weight'), this.settings.waveformWeightLevel, 1, 9, '',
          (v: number) => { this.settings.waveformWeightLevel = v; })
        this.toggleRow($r('app.string.aod_protocol'), this.settings.showProtocol,
          (v: boolean) => { this.settings.showProtocol = v; })
        this.toggleRow($r('app.string.aod_receive_rate'), this.settings.showReceiveRate,
          (v: boolean) => { this.settings.showReceiveRate = v; })
        this.toggleRow($r('app.string.aod_buffer'), this.settings.showBuffer,
          (v: boolean) => { this.settings.showBuffer = v; })
        this.toggleRow($r('app.string.aod_loss'), this.settings.showLoss,
          (v: boolean) => { this.settings.showLoss = v; })
        this.toggleRow($r('app.string.aod_audio_format'), this.settings.showAudioFormat,
          (v: boolean) => { this.settings.showAudioFormat = v; })
        this.toggleRow($r('app.string.aod_underruns'), this.settings.showUnderruns,
          (v: boolean) => { this.settings.showUnderruns = v; })
        this.toggleRow($r('app.string.aod_output_engine'), this.settings.showOutputEngine,
          (v: boolean) => { this.settings.showOutputEngine = v; })
      }.width('100%').padding({ left: 24, right: 24, bottom: 24 })
    }.width('100%')
  }

  fontName(): ResourceStr {
    switch (this.settings.clockFont) {
      case 1: return $r('app.string.aod_sans');
      case 2: return $r('app.string.aod_serif');
      case 3: return $r('app.string.aod_mono');
      default: return 'Rubik';
    }
  }

  build() {
    Column() {
      Row({ space: 4 }) {
        Button({ type: ButtonType.Circle }) {
          SymbolGlyph($r('sys.symbol.chevron_backward'))
            .fontSize(24)
            .fontColor([$r('sys.color.ohos_id_color_text_primary')])
        }
        .width(48)
        .height(48)
        .backgroundColor(Color.Transparent)
        .accessibilityText($r('app.string.back'))
        .onClick(() => {
          this.getUIContext().getRouter().back();
        })

        Text($r('app.string.settings'))
          .fontSize(20)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('sys.color.ohos_id_color_text_primary'))
      }
      .width('100%')
      .height(56)
      .padding({ left: 8, right: 16 })

      Scroll() {
        Column({ space: 8 }) {
          this.toggleRow($r('app.string.aod_auto'), this.settings.enabled,
            (v: boolean) => { this.settings.enabled = v; })
          Button($r('app.string.aod_open')).width('100%')
            .onClick(() => { this.getUIContext().getRouter().pushUrl({ url: 'pages/Aod' }); })
          Button($r('app.string.aod_options')).width('100%').buttonStyle(ButtonStyleMode.NORMAL)
            .onClick(() => { this.optionsOpen = true; })
            .bindSheet($$this.optionsOpen, this.options(), {
              detents: [SheetSize.MEDIUM, SheetSize.LARGE], title: { title: $r('app.string.aod_options') }
            })
          Text($r('app.string.paired_pcs')).fontSize(18).fontWeight(FontWeight.Medium)
            .margin({ top: 16 })
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
                    .onClick(() => {
                      anpod.forgetBinding(hostId);
                      this.load();
                    })
                }
                .width('100%')
                .height(64)
                .padding({ left: 24, right: 16 })
              }, (hostId: string) => hostId)
            }
            .borderRadius(24)
            .backgroundColor($r('app.color.card_bg'))
            .margin({ top: 8 })
          }
        }.width('100%').padding(16).alignItems(HorizontalAlign.Start)
      }.layoutWeight(1).width('100%')
    }
    .width('100%')
    .height('100%')
    .alignItems(HorizontalAlign.Start)
    .backgroundColor($r('sys.color.ohos_id_color_background'))
  }
}
