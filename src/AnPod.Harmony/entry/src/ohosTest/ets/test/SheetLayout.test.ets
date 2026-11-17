import { describe, expect, it } from '@ohos/hypium';
import { isWideWindow, sheetType } from '../../../main/ets/model/SheetLayout';

export default function sheetLayoutTest() {
  describe('SheetLayout', () => {
    it('sheets_float_centered_from_600vp', 0, () => {
      expect(isWideWindow(599.9)).assertFalse();
      expect(isWideWindow(600)).assertTrue();
      expect(sheetType(360)).assertEqual(SheetType.BOTTOM);
      expect(sheetType(599)).assertEqual(SheetType.BOTTOM);
      expect(sheetType(600)).assertEqual(SheetType.CENTER);
      expect(sheetType(1280)).assertEqual(SheetType.CENTER);
    });
  });
}
