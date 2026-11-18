import { describe, expect, it } from '@ohos/hypium';
import { hdsMaterial } from '@kit.UIDesignKit';
import { MaterialKind, pickMaterial } from '../../../main/ets/model/Material';

export default function materialTest() {
  describe('Material', () => {
    it('picks_immersive_then_adaptive_then_blur', 0, () => {
      const both = [hdsMaterial.MaterialType.ADAPTIVE, hdsMaterial.MaterialType.IMMERSIVE];
      expect(pickMaterial(both, true)).assertEqual(MaterialKind.IMMERSIVE);
      // The ArkUI device check must agree: a policy list alone is not enough.
      expect(pickMaterial(both, false)).assertEqual(MaterialKind.ADAPTIVE);
      expect(pickMaterial([hdsMaterial.MaterialType.ADAPTIVE], true)).assertEqual(MaterialKind.ADAPTIVE);
      expect(pickMaterial([hdsMaterial.MaterialType.NONE], true)).assertEqual(MaterialKind.BLUR);
      expect(pickMaterial([], false)).assertEqual(MaterialKind.BLUR);
    });
  });
}
