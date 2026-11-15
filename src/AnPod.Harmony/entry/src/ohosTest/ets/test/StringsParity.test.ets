import { describe, it, expect } from '@ohos/hypium';
import base from '../../../main/resources/base/element/string.json';
import zh from '../../../main/resources/zh_CN/element/string.json';
import en from '../../../main/resources/en_US/element/string.json';

interface StringEntry { name: string; value: string; }
interface StringFile { string: StringEntry[]; }

function keys(file: StringFile): string {
  return file.string.map((e: StringEntry) => e.name).sort().join(',');
}

export default function stringsParityTest() {
  describe('StringsParity', () => {
    it('string_json_key_sets_match', 0, () => {
      expect(keys(zh as StringFile)).assertEqual(keys(base as StringFile));
      expect(keys(en as StringFile)).assertEqual(keys(base as StringFile));
    });
  });
}
