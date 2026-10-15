// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { SwipeActions } from "./SwipeActions";
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement, root: Root;
const open = vi.fn(),
  pin = vi.fn(),
  hide = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  act(() =>
    root.render(
      <SwipeActions
        leading={{ label: "Pin", run: pin }}
        trailing={{ label: "Hide", run: hide }}
      >
        <button className="assignment" onClick={open}>
          Assignment
        </button>
      </SwipeActions>,
    ),
  );
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});
const gesture = (type: string, x: number, y = 100) =>
  act(() => {
    const event = new Event(type, { bubbles: true, cancelable: true });
    Object.assign(event, {
      clientX: x,
      clientY: y,
      pointerId: 1,
      pointerType: "touch",
      isPrimary: true,
    });
    host.querySelector(".assignment")!.dispatchEvent(event);
  });
const click = (selector: string) =>
  act(() => (host.querySelector(selector) as HTMLElement).click());
it("opens on one tap, without exposing an action first", () => {
  gesture("pointerdown", 100);
  gesture("pointerup", 100);
  click(".assignment");
  expect(open).toHaveBeenCalledTimes(1);
  expect(pin).not.toHaveBeenCalled();
  expect(hide).not.toHaveBeenCalled();
});
it("reveals pin to the right and suppresses the click following a swipe", () => {
  gesture("pointerdown", 100);
  gesture("pointermove", 180);
  gesture("pointerup", 180);
  click(".assignment");
  expect(open).not.toHaveBeenCalled();
  expect(host.querySelector(".leading")?.getAttribute("aria-hidden")).toBe(
    "false",
  );
  click(".leading");
  expect(pin).toHaveBeenCalledOnce();
  expect(hide).not.toHaveBeenCalled();
});
it("reveals hide to the left, while cancellation and vertical scrolling perform no action", () => {
  gesture("pointerdown", 200);
  gesture("pointermove", 195, 170);
  gesture("pointercancel", 195, 170);
  expect(
    host.querySelector<HTMLElement>(".swipeContent")?.style.transform,
  ).toBe("translateX(0px)");
  expect(open).not.toHaveBeenCalled();
  gesture("pointerdown", 200);
  gesture("pointermove", 100);
  gesture("pointerup", 100);
  click(".assignment");
  expect(open).not.toHaveBeenCalled();
  click(".trailing");
  expect(hide).toHaveBeenCalledOnce();
});
it("leaves the screen edge to back navigation", () => {
  gesture("pointerdown", 10);
  gesture("pointermove", 100);
  gesture("pointerup", 100);
  expect(host.querySelector(".leading")?.getAttribute("aria-hidden")).toBe(
    "true",
  );
  expect(pin).not.toHaveBeenCalled();
});
