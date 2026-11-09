import { describe, expect, it } from "vitest";
import { en } from "./strings.en";
import { zhCN } from "./strings.zh-CN";

const shape = (value: unknown): unknown => {
  if (typeof value === "function") return `fn/${value.length}`;
  if (value && typeof value === "object")
    return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, item]) => [key, shape(item)]));
  return typeof value;
};

describe("strings", () => {
  it("keeps zh-CN and en keys and formatter arity in sync", () => {
    expect(shape(en)).toEqual(shape(zhCN));
  });
});
