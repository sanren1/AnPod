import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';
import anpod from 'libanpod_core.so';

const DOMAIN = 0x0A90;
const TAG = 'AnPodProbe';

// Task 1 probe: logs want.parameters types and the native probe report.
function runProbe(want: Want): void {
  const params = want.parameters as Record<string, Object> | undefined;
  const port = params?.['port'];
  const token = params?.['launchToken'];
  hilog.info(DOMAIN, TAG, 'want port=%{public}s (%{public}s) launchToken=%{public}s (%{public}s)',
    String(port), typeof port, String(token), typeof token);
  const report = anpod.probe(port === undefined ? 0 : Number(port));
  for (const line of report.split('\n')) {
    hilog.info(DOMAIN, TAG, '%{public}s', line);
  }
}

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    runProbe(want);
  }

  onNewWant(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    runProbe(want);
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index');
  }
}
