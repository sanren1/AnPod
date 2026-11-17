import { AbilityConstant, UIAbility, Want, wantAgent } from '@kit.AbilityKit';
import { avSession } from '@kit.AVSessionKit';
import { backgroundTaskManager } from '@kit.BackgroundTasksKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';
import { parseLaunchParams } from '../model/LaunchParams';
import { ReceiverSession } from '../model/ReceiverSession';

const DOMAIN = 0xA0D0;

export default class EntryAbility extends UIAbility {
  private avSession?: avSession.AVSession;
  private live = false;
  // Start / end of the background hold run strictly in order.
  private background: Promise<void> = Promise.resolve();

  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    ReceiverSession.initialize(this.context, { live: () => this.setLive(true), ended: () => this.setLive(false) });
    if (parseLaunchParams(want.parameters) === undefined) {
      ReceiverSession.restore();
    } else {
      this.launch(want);
    }
  }

  onNewWant(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    this.launch(want);
  }

  // Closing the app is a manual disconnect: USER_STOPPED, the PC does not reconnect (wired or wireless).
  onDestroy(): void {
    ReceiverSession.closeApp();
    this.setLive(false);
  }

  onForeground(): void {
    AppStorage.setOrCreate('aodForeground', true);
  }

  onBackground(): void {
    AppStorage.setOrCreate('aodForeground', false);
  }

  onWindowStageDestroy(): void {
    AppStorage.setOrCreate('aodForeground', false);
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index');
  }

  // Started by the PC (hdc shell aa start --ps port/hostId/launchToken); anything invalid is ignored.
  private launch(want: Want): void {
    const params = parseLaunchParams(want.parameters);
    if (params === undefined) {
      return;
    }
    ReceiverSession.start(params);
  }

  private setLive(live: boolean): void {
    if (live === this.live) {
      return;
    }
    this.live = live;
    this.background = this.background.then(() => live ? this.holdBackground() : this.releaseBackground());
  }

  private async holdBackground(): Promise<void> {
    try {
      const session = await avSession.createAVSession(this.context, 'AnPod', 'audio');
      this.avSession = session;
      await session.setAVMetadata({ assetId: 'anpod', title: 'AnPod' });
      await session.activate();
      const agent = await wantAgent.getWantAgent({
        wants: [{ bundleName: this.context.abilityInfo.bundleName, abilityName: this.context.abilityInfo.name }],
        actionType: wantAgent.OperationType.START_ABILITY,
        requestCode: 0,
        actionFlags: [wantAgent.WantAgentFlags.UPDATE_PRESENT_FLAG]
      });
      await backgroundTaskManager.startBackgroundRunning(this.context,
        backgroundTaskManager.BackgroundMode.AUDIO_PLAYBACK, agent);
    } catch (e) {
      hilog.error(DOMAIN, 'AnPod', 'background hold failed: %{public}s', JSON.stringify(e));
    }
  }

  private async releaseBackground(): Promise<void> {
    try {
      await backgroundTaskManager.stopBackgroundRunning(this.context);
    } catch (e) {
      hilog.warn(DOMAIN, 'AnPod', 'stopBackgroundRunning: %{public}s', JSON.stringify(e));
    }
    const session = this.avSession;
    this.avSession = undefined;
    if (session !== undefined) {
      try {
        await session.deactivate();
        await session.destroy();
      } catch (e) {
        hilog.warn(DOMAIN, 'AnPod', 'AVSession release: %{public}s', JSON.stringify(e));
      }
    }
  }
}
