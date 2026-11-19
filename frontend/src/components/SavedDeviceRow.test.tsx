import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { strings } from "../strings";
import { SavedDeviceRow } from "./SavedDeviceRow";
import type { ComponentProps } from "react";

const noop = () => {};
const render = (props: Partial<ComponentProps<typeof SavedDeviceRow>> = {}) => renderToStaticMarkup(<ul>
  <SavedDeviceRow label="MatePad" online={false} offer={null} error={null} busy={false} connecting={false} onConnect={noop} onAccept={noop} onDecline={noop} {...props} />
</ul>);

describe("SavedDeviceRow", () => {
  it("online: green dot, status text and a primary connect button", () => {
    const html = render({ online: true });
    expect(html).toContain("MatePad");
    expect(html).toContain(strings.network.deviceOnline);
    expect(html).toContain("data-online");
    expect(html).toMatch(new RegExp(`class="primary-button"[^>]*>${strings.network.connectDevice}<`));
  });

  it("not found: neutral dot and a secondary connect button that is still available", () => {
    const html = render();
    expect(html).toContain(strings.network.deviceNotFound);
    expect(html).not.toContain("data-online");
    expect(html).toMatch(new RegExp(`class="dialog-secondary"[^>]*>${strings.network.connectDevice}<`));
    expect(html).not.toContain("disabled");
  });

  it("request: shows accept and decline instead of connect", () => {
    const html = render({ online: true, offer: { id: "o1", endpoint: "192.0.2.5:4000" } });
    expect(html).toContain(strings.network.deviceRequest);
    expect(html).toContain(strings.network.acceptRequest);
    expect(html).toContain(strings.network.declineRequest);
    expect(html).not.toContain(`>${strings.network.connectDevice}<`);
  });

  it("failed: one-line reason, retry button and collapsed details", () => {
    const html = render({ error: { summary: strings.network.rowNotFound, details: "raw detail" } });
    expect(html).toContain(strings.network.rowNotFound);
    expect(html).toContain(`>${strings.network.retryConnect}<`);
    expect(html).toContain("<details>");
    expect(html).not.toContain("<details open");
    expect(html).toContain("raw detail");
  });

  it("failed without details has no details block; busy disables the row", () => {
    const html = render({ error: { summary: strings.network.rowNotFound, details: "" }, busy: true, connecting: true });
    expect(html).not.toContain("<details");
    expect(html).toContain("disabled");
    expect(html).toContain(strings.network.connectingDevice);
  });

});
