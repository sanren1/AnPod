import { describe, it, expect } from '@ohos/hypium';
import { parseLaunchParams } from '../../../main/ets/model/LaunchParams';

const HOST = '0123456789abcdef0123456789abcdef';
const TOKEN = 'fedcba9876543210fedcba9876543210';

function params(port: Object, hostId: Object, launchToken: Object): Record<string, Object> {
  const p: Record<string, Object> = {};
  p['port'] = port;
  p['hostId'] = hostId;
  p['launchToken'] = launchToken;
  return p;
}

export default function entryParamsTest() {
  describe('EntryParams', () => {
    it('entry_params_validation', 0, () => {
      // aa start --ps delivers strings; a number is accepted too.
      const ok = parseLaunchParams(params('47000', HOST, TOKEN));
      expect(ok?.port).assertEqual(47000);
      expect(ok?.hostId).assertEqual(HOST);
      expect(ok?.launchToken).assertEqual(TOKEN);
      expect(parseLaunchParams(params(40000, HOST, TOKEN))?.port).assertEqual(40000);
      expect(parseLaunchParams(params('49999', HOST, TOKEN))?.port).assertEqual(49999);

      const bad: Record<string, Object>[] = [
        params('39999', HOST, TOKEN), params('50000', HOST, TOKEN), params('4700a', HOST, TOKEN),
        params('047000', HOST, TOKEN), params(' 47000', HOST, TOKEN), params(47000.5, HOST, TOKEN),
        params('47000', HOST.toUpperCase(), TOKEN), params('47000', HOST.slice(1), TOKEN),
        params('47000', HOST + '0', TOKEN), params('47000', HOST, 'g' + TOKEN.slice(1)),
        params('47000', HOST, TOKEN.slice(2)), params('47000', HOST, 7),
      ];
      bad.forEach((p: Record<string, Object>, i: number) => {
        expect(parseLaunchParams(p) === undefined ? '' : `case ${i}`).assertEqual('');
      });
      expect(parseLaunchParams(undefined) === undefined).assertTrue();
      const missing: Record<string, Object> = {};
      missing['port'] = '47000';
      expect(parseLaunchParams(missing) === undefined).assertTrue();
    });
  });
}
