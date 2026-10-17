import { en } from "./strings.en";
import { zhCN } from "./strings.zh-CN";

export type Strings = typeof zhCN;

const locale = typeof navigator === "undefined" ? "zh" : navigator.language;
export const strings: Strings = locale.toLowerCase().startsWith("zh") ? zhCN : en;
