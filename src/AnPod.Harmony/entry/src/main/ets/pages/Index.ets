import { KEY_CODE, KEY_ERROR, KEY_FORMAT, KEY_PC_CONFIRMED, KEY_STATE, ReceiverSession } from '../model/ReceiverSession';

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
  @StorageProp(KEY_STATE) coreState: string = 'idle';
  @StorageProp(KEY_FORMAT) formatText: string = '';
  @StorageProp(KEY_ERROR) errorMessage: string = '';
  @StorageProp(KEY_CODE) @Watch('onPairingCode') pairingCode: string = '';
  @State wide: boolean = true;
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

  live(): boolean {
    return this.coreState !== 'idle' && this.coreState !== 'stopped' && this.coreState !== 'failed';
  }

  statusText(): Resource {
    switch (this.coreState) {
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
      case 'failed':
        return $r('app.string.state_failed');
      default:
        return $r('app.string.state_idle');
    }
  }

  // Known causes get localized copy; anything else shows the native message as is.
  errorText(): ResourceStr {
    return this.errorMessage.endsWith('low latency unsupported') ? $r('app.string.error_low_latency') :
      this.errorMessage;
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
      Text($r('app.string.tab_wired'))
        .fontSize(16)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('sys.color.ohos_id_color_text_primary_contrary'))
        .backgroundColor($r('sys.color.ohos_id_color_emphasize'))
        .borderRadius(18)
        .height(36)
        .padding({ left: 20, right: 20 })
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
