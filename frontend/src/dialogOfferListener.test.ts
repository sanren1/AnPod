import { describe, expect, it, vi } from "vitest";
import { createDialogOfferListener } from "./dialogOfferListener";

const setup = () => {
  const start = vi.fn(async () => true);
  const stop = vi.fn();
  return { start, stop, listener: createDialogOfferListener() };
};

describe("dialog offer listener", () => {
  it("starts when none runs and stops it again on close", async () => {
    const { listener, start, stop } = setup();
    await listener.ensure("a|b", false, start, stop);
    expect(start).toHaveBeenCalledTimes(1);
    listener.release(stop);
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it("leaves a listener that was already running alone", async () => {
    const { listener, start, stop } = setup();
    await listener.ensure("a|b", true, start, stop);
    listener.release(stop);
    expect(start).not.toHaveBeenCalled();
    expect(stop).not.toHaveBeenCalled();
  });

  it("does not restart its own listener for the same choice but follows a changed choice", async () => {
    const { listener, start, stop } = setup();
    await listener.ensure("a|b", false, start, stop);
    await listener.ensure("a|b", true, start, stop);
    expect(start).toHaveBeenCalledTimes(1);
    await listener.ensure("c|b", true, start, stop);
    expect(start).toHaveBeenCalledTimes(2);
  });

  it("does not own a listener whose start failed", async () => {
    const { listener, stop } = setup();
    await listener.ensure("a|b", false, async () => false, stop);
    listener.release(stop);
    expect(stop).not.toHaveBeenCalled();
  });

  it("stops a listener that finished starting after the dialog closed", async () => {
    const { listener, stop } = setup();
    await listener.ensure("a|b", false, async () => { listener.release(stop); return true; }, stop);
    expect(stop).toHaveBeenCalledTimes(1);
    listener.release(stop);
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it("owns a listener again after the dialog is reopened", async () => {
    const { listener, start, stop } = setup();
    listener.release(stop);
    listener.open();
    await listener.ensure("a|b", false, start, stop);
    listener.release(stop);
    expect(stop).toHaveBeenCalledTimes(1);
  });
});

describe("dialog offer listener commands", () => {
  it("sends allowed:false on close only for a listener the dialog started", async () => {
    const invoke = vi.fn(async (_command: string, _args: Record<string, unknown>) => ({}));
    const start = async () => { await invoke("set_wireless_connection_offers", { localAddress: "192.0.2.1", allowed: true, bindingReference: "b" }); return true; };
    const stop = () => void invoke("set_wireless_connection_offers", { localAddress: "", allowed: false });
    const fresh = createDialogOfferListener();
    await fresh.ensure("k", false, start, stop);
    fresh.release(stop);
    expect(invoke.mock.calls.map(call => call[1].allowed)).toEqual([true, false]);
    invoke.mockClear();
    const preexisting = createDialogOfferListener();
    await preexisting.ensure("k", true, start, stop);
    preexisting.release(stop);
    expect(invoke).not.toHaveBeenCalled();
  });
});
