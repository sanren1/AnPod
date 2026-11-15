import anpod from 'libanpod_core.so';

@Entry
@Component
struct PairedPcs {
  @State hostIds: string[] = [];
  @State unavailable: boolean = false;

  aboutToAppear(): void {
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

        Text($r('app.string.paired_pcs'))
          .fontSize(20)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('sys.color.ohos_id_color_text_primary'))
      }
      .width('100%')
      .height(56)
      .padding({ left: 8, right: 16 })

      if (this.unavailable) {
        this.message($r('app.string.store_unavailable'))
      } else if (this.hostIds.length === 0) {
        this.message($r('app.string.no_paired_pcs'))
      } else {
        List() {
          ForEach(this.hostIds, (hostId: string) => {
            ListItem() {
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
            }
          }, (hostId: string) => hostId)
        }
        .divider({
          strokeWidth: 1,
          color: $r('sys.color.ohos_id_color_list_separator'),
          startMargin: 24,
          endMargin: 16
        })
        .borderRadius(24)
        .backgroundColor($r('app.color.card_bg'))
        .margin({ left: 16, right: 16, top: 8 })
        .layoutWeight(1)
      }
    }
    .width('100%')
    .height('100%')
    .alignItems(HorizontalAlign.Start)
    .backgroundColor($r('sys.color.ohos_id_color_background'))
  }
}
