import { uiMaterial } from '@kit.ArkUI';
import { hdsMaterial } from '@kit.UIDesignKit';
import { hilog } from '@kit.PerformanceAnalysisKit';

export enum MaterialKind {
  IMMERSIVE,
  ADAPTIVE,
  BLUR
}

// IMMERSIVE needs both the HDS policy list and the ArkUI device check; ADAPTIVE only exists as an HDS policy
// (no generic modifier), so surfaces treat it like BLUR, the system blur over an almost opaque fill.
export function pickMaterial(types: number[], immersiveApi: boolean): MaterialKind {
  if (immersiveApi && types.includes(hdsMaterial.MaterialType.IMMERSIVE)) {
    return MaterialKind.IMMERSIVE;
  }
  return types.includes(hdsMaterial.MaterialType.ADAPTIVE) ? MaterialKind.ADAPTIVE : MaterialKind.BLUR;
}

// Attributes a surface sets: a fill, plus the immersive material or the system blur.
export class Surface {
  material: uiMaterial.Material | undefined = undefined;
  fill: ResourceColor = Color.Transparent;
  blur: BlurStyle = BlurStyle.NONE;
}

function detect(): MaterialKind {
  let types: number[] = [];
  let immersiveApi = false;
  try {
    types = hdsMaterial.getSystemMaterialTypes();
    immersiveApi = uiMaterial.isImmersiveMaterialSupported();
    hilog.info(0xA0D0, 'AnPod', 'material types=%{public}s immersiveApi=%{public}s level=%{public}d',
      JSON.stringify(types), String(immersiveApi), uiMaterial.getGlobalMaterialLevel());
  } catch (e) {
    hilog.warn(0xA0D0, 'AnPod', 'material query failed: %{public}s', JSON.stringify(e));
  }
  return pickMaterial(types, immersiveApi);
}

const KIND: MaterialKind = detect();

function surface(style: uiMaterial.ImmersiveStyle, fallbackFill: ResourceColor): Surface {
  const s = new Surface();
  // The fill always stays: where the system draws no material (the emulator) the surface is still visible.
  s.fill = fallbackFill;
  if (KIND === MaterialKind.IMMERSIVE) {
    s.material = new uiMaterial.ImmersiveMaterial({ style: style, materialColor: fallbackFill });
  } else {
    s.blur = BlurStyle.COMPONENT_REGULAR;
  }
  return s;
}

export const BAR_SURFACE: Surface = surface(uiMaterial.ImmersiveStyle.THIN, $r('app.color.bar_glass'));
export const CARD_SURFACE: Surface = surface(uiMaterial.ImmersiveStyle.REGULAR, $r('app.color.card_glass'));
