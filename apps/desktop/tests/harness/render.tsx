import { act, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { expect, vi } from "vitest";

/**
 * Mounting helpers.
 *
 * The application polls Core, so a suite must both render inside `act` and wait
 * for the reads it triggered. `mount` owns the root and `until` owns the wait;
 * suites then read the rendered DOM instead of reaching into component state.
 */

export type Mounted = {
  host: HTMLElement;
  root: Root;
  /** Unmounts inside `act` and removes the host element. */
  unmount(): Promise<void>;
};

export async function mount(element: ReactElement): Promise<Mounted> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(element));
  return {
    host,
    root,
    async unmount() {
      await act(async () => root.unmount());
      host.remove();
    },
  };
}

/** Resolves after every pending microtask and React update settles. */
export async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** Waits for real time to pass inside `act`, for polling and animation. */
export async function advance(milliseconds: number): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, milliseconds));
  });
}

/** Retries an assertion until it passes or the timeout expires. */
export function until<T>(
  check: () => T,
  // Generous by default: a suite sharing the machine with a full workspace
  // build can take several seconds to settle its first render.
  options?: { timeout?: number },
): Promise<T> {
  return vi.waitFor(check, { timeout: options?.timeout ?? 10_000 });
}

export function textOf(host: ParentNode, selector: string): string[] {
  return [...host.querySelectorAll(selector)].map(
    (node) => node.textContent?.trim() ?? "",
  );
}

export function buttonByText(
  host: ParentNode,
  label: string,
): HTMLButtonElement | undefined {
  return [...host.querySelectorAll("button")].find(
    (button) => (button.textContent ?? "").trim() === label,
  );
}

/** Types into a controlled input the way React's synthetic events expect. */
export async function typeInto(
  input: HTMLInputElement | HTMLTextAreaElement,
  value: string,
): Promise<void> {
  await act(async () => {
    const prototype =
      input instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(
      input,
      value,
    );
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

export async function click(target: Element | null | undefined): Promise<void> {
  if (!target) throw new Error("click() received no element.");
  await act(async () => {
    (target as HTMLElement).click();
  });
}

/** Opens the real portalled combobox and picks an enabled option by its value. */
export async function chooseOption(
  control: HTMLElement,
  value: string,
): Promise<void> {
  await click(control);
  const option = await until(() => {
    const list = document.getElementById(
      control.getAttribute("aria-controls") ?? "",
    );
    const option = [
      ...(list?.querySelectorAll<HTMLElement>('[role="option"]') ?? []),
    ].find((item) => item.getAttribute("value") === value);
    if (!option) throw new Error(`Option ${value} is not available.`);
    if (option.hasAttribute("data-combobox-disabled"))
      throw new Error(`Option ${value} is disabled.`);
    return option;
  });
  await click(option);
}

/**
 * Fails a test on unexpected `console.error`, ignoring React's act() notice for
 * asynchronous polling that the application itself cannot wrap.
 */
export function watchConsoleErrors() {
  const spy = vi.spyOn(console, "error").mockImplementation(() => {});
  return {
    expectClean() {
      const unexpected = spy.mock.calls.filter(
        (call) => !String(call[0]).includes("not wrapped in act"),
      );
      expect(unexpected).toEqual([]);
    },
    restore() {
      spy.mockRestore();
    },
  };
}
