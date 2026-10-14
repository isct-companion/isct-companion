// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

// Scripts the native side injects into every school webview at document start.
const script = (name: string) =>
  readFileSync(
    resolve(process.cwd(), "crates/isct-core/src/school_browser/assets", name),
    "utf8",
  );

const run = (source: string) => new Function(source)();

describe("school link routing", () => {
  const open = vi.fn();
  let prevented: boolean | undefined;

  beforeAll(() => {
    window.open = open;
    run(script("school-links.js"));
    // Registered last: records the outcome, then stops jsdom navigating.
    window.addEventListener("click", (event) => {
      prevented = event.defaultPrevented;
      event.preventDefault();
    });
  });

  afterEach(() => {
    open.mockClear();
    document.body.replaceChildren();
  });

  const click = (attributes: Record<string, string>, init?: MouseEventInit) => {
    const link = document.createElement("a");
    for (const [key, value] of Object.entries(attributes))
      link.setAttribute(key, value);
    link.textContent = "course";
    document.body.append(link);
    link.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, ...init }),
    );
  };

  it("sends cross-origin new-window links through window.open", () => {
    // 教務Web's course names; the app routes them to the Syllabus service.
    click({
      href: "https://syllabus.s.isct.ac.jp/courses?jwc=202512345",
      target: "_blank",
    });
    expect(open).toHaveBeenCalledWith(
      "https://syllabus.s.isct.ac.jp/courses?jwc=202512345",
      "_blank",
      "noopener",
    );
    expect(prevented).toBe(true);
    click({ href: "https://www.isct.ac.jp/ja/a.pdf", target: "_BLANK" });
    expect(open).toHaveBeenLastCalledWith(
      "https://www.isct.ac.jp/ja/a.pdf",
      "_blank",
      "noopener",
    );
  });

  it("leaves every other click to the page and WebKit", () => {
    click({ href: "https://syllabus.s.isct.ac.jp/" });
    click({ href: "/same-origin", target: "_blank" });
    click({ href: "mailto:office@example.com", target: "_blank" });
    click(
      { href: "https://syllabus.s.isct.ac.jp/", target: "_blank" },
      { metaKey: true },
    );
    const link = document.createElement("a");
    link.href = "https://syllabus.s.isct.ac.jp/";
    link.target = "_blank";
    link.addEventListener("click", (event) => event.preventDefault());
    document.body.append(link);
    link.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true }),
    );
    expect(open).not.toHaveBeenCalled();
  });
});

describe("教務Web popup overlay", () => {
  const nativeOpen = vi.fn(() => null);

  beforeAll(() => {
    window.open = nativeOpen;
    // The script only runs on https://kyomu0…; jsdom serves http://localhost.
    run(
      script("kyomu-popup.js")
        .replace(
          'location.protocol !== "https:"',
          'location.protocol !== "http:"',
        )
        .replace('"kyomu0.gakumu.titech.ac.jp"', `"${location.hostname}"`),
    );
  });

  const overlays = () => [
    ...document.querySelectorAll<HTMLIFrameElement>(
      "iframe[data-isct-companion-popup]",
    ),
  ];

  it("shows same-origin popups in a named frame that knows its opener", async () => {
    const popup = window.open(
      "/Titech/PID1_3_1.aspx?cps=1",
      "PID13_1aspx",
      "width=600, height=400",
    );
    const [frame] = overlays();
    expect(frame.name).toBe("PID13_1aspx");
    expect(frame.src).toBe(
      new URL("/Titech/PID1_3_1.aspx?cps=1", location.href).href,
    );
    expect(
      (frame as HTMLIFrameElement & { isctCompanionOpener?: Window })
        .isctCompanionOpener,
    ).toBe(window);
    expect(popup).toBe(frame.contentWindow);
    // The same window name reuses the overlay, like a named popup window.
    window.open("/Titech/PID1_3_1.aspx?cps=2", "PID13_1aspx");
    expect(overlays()).toHaveLength(1);
    expect(nativeOpen).not.toHaveBeenCalled();

    (
      window as Window & {
        __isctCompanionPopups: { close(frame: Element): void };
      }
    ).__isctCompanionPopups.close(frame);
    await new Promise((resolve) => setTimeout(resolve));
    expect(overlays()).toHaveLength(0);
  });

  it("keeps the app's routing for other origins and named targets", () => {
    window.open(
      "https://syllabus.s.isct.ac.jp/courses?jwc=202512345",
      "_blank",
      "noopener",
    );
    window.open("/Titech/Default.aspx", "_top");
    expect(nativeOpen).toHaveBeenCalledTimes(2);
    expect(overlays()).toHaveLength(0);
  });
});
