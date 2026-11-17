import { common } from '@kit.AbilityKit';
import { loadAodSettings } from '../model/AodSettings';
import anpod from 'libanpod_core.so';
import { KEY_CODE, KEY_ERROR, KEY_FORMAT, KEY_HOST, KEY_PC_CONFIRMED, KEY_STATE, ReceiverSession } from '../model/ReceiverSession';

const CARD_RADIUS = 24;

@CustomDialog
struct PairingDialog {
  controller?: CustomDialogController;
  @StorageProp(KEY_CODE) code: string = '';
  @StorageProp(KEY_PC_CONFIRMED) pcConfirmed: boolean = false;

  build() {
    Column({ space: 16 }) {
      Text($r('app.string.pairing_title'))
        .fontSize(18)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('sys.color.ohos_id_color_text_primary'))
      Text(this.code)
        .fontSize(44)
        .fontWeight(FontWeight.Bold)
        .letterSpacing(8)
        .fontFamily('monospace')
        .fontColor($r('sys.color.ohos_id_color_text_primary'))
      // The PC confirms first, then the tablet (spec section 7).
      Text($r('app.string.pairing_wait_pc'))
        .fontSize(14)
        .fontColor($r('sys.color.ohos_id_color_text_secondary'))
        .visibility(this.pcConfirmed ? Visibility.Hidden : Visibility.Visible)
      Row({ space: 12 }) {
        Button($r('app.string.cancel'))
          .buttonStyle(ButtonStyleMode.NORMAL)
          .height(48)
          .layoutWeight(1)
          .onClick(() => ReceiverSession.answerPairing(false))
        Button($r('app.string.confirm'))
          .buttonStyle(ButtonStyleMode.EMPHASIZED)
          .height(48)
          .layoutWeight(1)
          .enabled(this.pcConfirmed)
          .onClick(() => ReceiverSession.answerPairing(true))
      }
      .width('100%')
    }
    .padding(24)
  }
}

@Entry
@Component
struct Index {
  @StorageProp(KEY_STATE) @Watch('maybeEnterAod') coreState: string = 'idle';
  @StorageProp(KEY_FORMAT) formatText: string = '';
  @StorageProp(KEY_ERROR) errorMessage: string = '';
  @StorageProp(KEY_HOST) activeHost: string = '';
  @StorageProp(KEY_CODE) @Watch('onPairingCode') pairingCode: string = '';
  @StorageProp('transportPage') @Watch('reloadBindings') transport: string = 'wired';
  @State hosts: string[] = [];
  @State bindingError: boolean = false;
  @State wide: boolean = true;
  @StorageProp('aodForeground') @Watch('maybeEnterAod') foreground: boolean = false;
  private pageVisible: boolean = false;
  private dialogOpen: boolean = false;
  private dialog: CustomDialogController = new CustomDialogController({
    builder: PairingDialog(),
    autoCancel: false,
    // Back and outside taps do not dismiss it: the user answers with a button.
    onWillDismiss: () => {}
  });

  onPairingCode(): void {
    if (this.pairingCode !== '' && !this.dialogOpen) {
      this.dialogOpen = true;
      this.dialog.open();
    } else if (this.pairingCode === '' && this.dialogOpen) {
      this.dialogOpen = false;
      this.dialog.close();
    }
  }

  aboutToAppear(): void {
    this.onPairingCode();
  }

  onPageHide(): void {
    this.pageVisible = false;
  }

  maybeEnterAod(): void {
    if (!this.pageVisible || !this.foreground || (this.coreState !== 'connected' && this.coreState !== 'playing') ||
      AppStorage.get<boolean>('aodDismissed') === true) return;
    const context = this.getUIContext().getHostContext() as common.UIAbilityContext;
    if (!loadAodSettings(context).enabled) return;
    AppStorage.setOrCreate('aodDismissed', true);
    this.getUIContext().getRouter().pushUrl({ url: 'pages/Aod' }).catch((error: Error) => {
      AppStorage.setOrCreate('aodDismissed', false);
      console.error('Cannot open AOD: ' + error.message);
    });
  }

  onPageShow(): void {
    this.pageVisible = true;
    this.reloadBindings();
    this.maybeEnterAod();
  }

  reloadBindings(): void {
    if (this.transport === 'wireless') {
      try {
        this.hosts = anpod.listBindings();
        this.bindingError = false;
      } catch (e) {
        this.hosts = [];
        this.bindingError = true;
      }
    }
  }

  live(): boolean {
    return this.coreState !== 'listening' && this.coreState !== 'idle' && this.coreState !== 'stopped' && this.coreState !== 'failed';
  }

  statusText(): Resource {
    if (this.transport === 'wireless' && this.hosts.length === 0 && !this.bindingError) {
      return $r('app.string.no_paired_pcs');
    }
    switch (this.coreState) {
      case 'listening':
        return $r('app.string.wireless_ready');
      case 'searching':
        return $r('app.string.wireless_searching');
      case 'connecting':
        return $r('app.string.state_connecting');
      case 'pairing':
        return $r('app.string.state_pairing');
      case 'connected':
        return $r('app.string.state_connected');
      case 'playing':
        return $r('app.string.state_playing');
      case 'paused':
        return $r('app.string.state_paused');
      case 'reconnecting':
        return $r('app.string.state_reconnecting');
      case 'failed':
        return $r('app.string.state_failed');
      default:
        return $r('app.string.state_idle');
    }
  }

  // Known causes get localized copy; anything else shows the native message as is.
  errorText(): ResourceStr {
    if (this.errorMessage.endsWith('low latency unsupported')) {
      return $r('app.string.error_low_latency');
    }
    switch (this.errorMessage) {
      case 'wireless discovery unavailable':
        return $r('app.string.error_discovery_unavailable');
      case 'wireless discovery send failed':
        return $r('app.string.error_discovery_send');
      case 'paired PC did not respond':
        return $r('app.string.error_pc_no_response');
      case 'binding store unavailable':
        return $r('app.string.store_unavailable');
      case 'cannot reach the PC on the hdc forward':
        return $r('app.string.error_hdc_unreachable');
      case 'pairing timed out':
        return $r('app.string.error_pairing_timeout');
      case 'pairing offer for an already paired PC':
        return $r('app.string.error_already_paired');
      default:
        return this.errorMessage;
    }
  }

  // Real state of one paired PC: only the session's own peer is ever connecting, reconnecting or connected.
  hostState(id: string): ResourceStr {
    if (id !== this.activeHost) {
      return this.live() ? '' : $r('app.string.pc_paired');
    }
    switch (this.coreState) {
      case 'searching':
      case 'connecting':
      case 'pairing':
        return $r('app.string.state_connecting');
      case 'reconnecting':
        return $r('app.string.state_reconnecting');
      case 'connected':
      case 'playing':
      case 'paused':
        return $r('app.string.state_connected');
      default:
        return $r('app.string.pc_paired');
    }
  }

  statusColor(): Resource {
    if (this.coreState === 'playing') {
      return $r('app.color.status_ok');
    }
    return this.coreState === 'failed' ? $r('app.color.status_error') : $r('app.color.status_idle');
  }

  @Builder
  statusCard() {
    Column({ space: 24 }) {
      Row({ space: 8 }) {
        Button($r('app.string.tab_wired'))
          .buttonStyle(this.transport === 'wired' ? ButtonStyleMode.EMPHASIZED : ButtonStyleMode.NORMAL)
          .height(48)
          .onClick(() => { if (this.transport !== 'wired') ReceiverSession.selectMode(false); })
        Button($r('app.string.tab_wireless'))
          .buttonStyle(this.transport === 'wireless' ? ButtonStyleMode.EMPHASIZED : ButtonStyleMode.NORMAL)
          .height(48)
          .onClick(() => { if (this.transport !== 'wireless' || this.coreState === 'stopped' ||
            this.coreState === 'failed') ReceiverSession.selectMode(true); })
      }
      Row({ space: 12 }) {
        Circle().width(12).height(12).fill(this.statusColor())
        Text(this.statusText())
          .fontSize(28)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('sys.color.ohos_id_color_text_primary'))
          .maxLines(1)
          .minFontSize(18)
          .maxFontSize(28)
          .layoutWeight(1)
      }
      .width('100%')

      if (this.transport === 'wireless') {
        if (this.bindingError) {
          Text($r('app.string.store_unavailable'))
            .fontColor($r('sys.color.ohos_id_color_text_secondary'))
        } else {
          // A live session is ended with Disconnect first, so a tap never drops it silently.
          ForEach(this.hosts, (id: string) => {
            Button() {
              Row() {
                Text($r('app.string.pc_label', id.slice(0, 4)))
                  .fontSize(16)
                  .fontColor($r('sys.color.ohos_id_color_text_primary'))
                Blank()
                Text(this.hostState(id))
                  .fontSize(14)
                  .fontColor($r('sys.color.ohos_id_color_text_secondary'))
              }
              .width('100%')
              .padding({ left: 16, right: 16 })
            }
            .buttonStyle(ButtonStyleMode.NORMAL)
            .width('100%')
            .height(48)
            .enabled(!this.live() || id === this.activeHost)
            .onClick(() => { if (!this.live()) ReceiverSession.wireless(id); })
          }, (id: string) => id)
        }
      }
      if (this.coreState === 'failed' && this.errorMessage !== '') {
        Text(this.errorText())
          .fontSize(14)
          .fontColor($r('sys.color.ohos_id_color_text_secondary'))
          .maxLines(3)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
      }
    }
    .width('100%')
    .alignItems(HorizontalAlign.Start)
    .padding(24)
    .borderRadius(CARD_RADIUS)
    .backgroundColor($r('app.color.card_bg'))
  }

  @Builder
  playbackCard() {
    Column({ space: 24 }) {
      if (this.formatText !== '') {
        Text(this.formatText)
          .fontSize(22)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('sys.color.ohos_id_color_text_primary'))
          .maxLines(1)
          .minFontSize(14)
          .maxFontSize(22)
          .width('100%')
          .textAlign(TextAlign.Center)
      }
      Button($r('app.string.disconnect'))
        .buttonStyle(ButtonStyleMode.NORMAL)
        .height(48)
        .width('100%')
        .onClick(() => ReceiverSession.disconnect())
    }
    .width('100%')
    .padding(24)
    .borderRadius(CARD_RADIUS)
    .backgroundColor($r('app.color.card_bg'))
  }

  build() {
    Column() {
      Row() {
        Text('AnPod')
          .fontSize(24)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('sys.color.ohos_id_color_text_primary'))
        Blank()
        Button({ type: ButtonType.Circle }) {
          SymbolGlyph($r('sys.symbol.gearshape'))
            .fontSize(24)
            .fontColor([$r('sys.color.ohos_id_color_text_primary')])
        }
        .width(48)
        .height(48)
        .backgroundColor(Color.Transparent)
        .accessibilityText($r('app.string.settings'))
        .onClick(() => {
          this.getUIContext().getRouter().pushUrl({ url: 'pages/PairedPcs' });
        })
      }
      .width('100%')
      .height(56)
      .padding({ left: 24, right: 12 })

      // The playback card exists only while a session is live; idle, the status card fills the width.
      if (this.wide) {
        Row({ space: 16 }) {
          Column() {
            this.statusCard()
          }
          .layoutWeight(3)

          if (this.live()) {
            Column() {
              this.playbackCard()
            }
            .layoutWeight(2)
          }
        }
        .width('100%')
        .alignItems(VerticalAlign.Top)
        .padding({ left: 16, right: 16, top: 8 })
      } else {
        Column({ space: 16 }) {
          this.statusCard()
          if (this.live()) {
            this.playbackCard()
          }
        }
        .width('100%')
        .padding({ left: 16, right: 16, top: 8 })
      }
    }
    .width('100%')
    .height('100%')
    .backgroundColor($r('sys.color.ohos_id_color_background'))
    .onAreaChange((_old: Area, area: Area) => {
      this.wide = Number(area.width) >= 600;
    })
  }
}
