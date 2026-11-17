// Windows at least this wide (vp) get the two-column main page and centered floating sheets.
const WIDE_WINDOW_VP = 600;

export function isWideWindow(width: number): boolean {
  return width >= WIDE_WINDOW_VP;
}

export function sheetType(width: number): SheetType {
  return isWideWindow(width) ? SheetType.CENTER : SheetType.BOTTOM;
}

// Secondary menus: a floating sheet with a title, the system close button and a dimmed mask that takes
// outside taps. EMBEDDED keeps the sheet on its own page, so a pushed page (AOD) covers it.
export function floatingSheet(title: ResourceStr, width: number, height: SheetSize, onDisappear?: () => void): SheetOptions {
  return {
    title: { title: title },
    preferType: sheetType(width),
    height: height,
    showClose: true,
    enableOutsideInteractive: false,
    mode: SheetMode.EMBEDDED,
    onDisappear: onDisappear
  };
}
