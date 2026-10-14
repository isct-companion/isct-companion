// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { scheduleQueries, useScheduleQuery } from "./core-query";
import { useFinderQuery } from "./finder-query";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const disposals: (() => void)[] = [];
afterEach(() => {
  disposals.splice(0).forEach((dispose) => dispose());
});
// This repo uses React's own act/createRoot for hook integration tests.
function renderHook<T, P = undefined>(
  hook: (props: P) => T,
  options?: { initialProps: P },
) {
  const host = document.createElement("div");
  const root = createRoot(host);
  const result = { current: undefined as T };
  let props = options?.initialProps as P;
  function Probe() {
    result.current = hook(props);
    return null;
  }
  const rerender = (next = props) => {
    props = next;
    act(() => root.render(<Probe />));
  };
  let mounted = true;
  const unmount = () => {
    if (mounted) {
      act(() => root.unmount());
      mounted = false;
    }
  };
  disposals.push(unmount);
  rerender();
  return { result, rerender, unmount };
}
async function waitFor(check: () => void) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  check();
}

const native = vi.hoisted(() => ({
  scheduleQuery: vi.fn(),
  finderQuery: vi.fn(),
  revision: "",
}));
vi.mock("./desktop-bridge", () => ({ getDesktopBridge: () => native }));
vi.mock("../store/useWorkspace", () => ({
  useWorkspace: (select: (state: { revision: string }) => unknown) =>
    select({ revision: native.revision }),
}));
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
beforeEach(() => {
  vi.resetAllMocks();
  native.revision = crypto.randomUUID();
});

describe("Core read projections", () => {
  it("shares a snapshot request between consumers and invalidates it on a new revision", async () => {
    native.scheduleQuery.mockResolvedValue({
      revision: native.revision,
      values: [{ kind: "declaredCredits", value: 6 }],
    });
    const { result, rerender } = renderHook(() => [
      useScheduleQuery<number>({ kind: "declaredCredits", term: "2026-3Q" }),
      useScheduleQuery<number>({ kind: "declaredCredits", term: "2026-3Q" }),
    ]);
    await waitFor(() =>
      expect(result.current.map((x) => x.value)).toEqual([6, 6]),
    );
    expect(native.scheduleQuery).toHaveBeenCalledTimes(1);
    native.revision = crypto.randomUUID();
    native.scheduleQuery.mockResolvedValue({
      revision: native.revision,
      values: [{ kind: "declaredCredits", value: 8 }],
    });
    rerender();
    await waitFor(() =>
      expect(result.current.map((x) => x.value)).toEqual([8, 8]),
    );
    expect(native.scheduleQuery).toHaveBeenCalledTimes(2);
  });
  it("a late response cannot replace the selected quarter", async () => {
    const old = deferred<unknown>();
    native.scheduleQuery.mockReturnValueOnce(old.promise).mockResolvedValue({
      revision: native.revision,
      values: [{ kind: "declaredCredits", value: 9 }],
    });
    const { result, rerender } = renderHook(
      ({ term }) => useScheduleQuery<number>({ kind: "declaredCredits", term }),
      { initialProps: { term: "2026-2Q" } },
    );
    rerender({ term: "2026-3Q" });
    await waitFor(() => expect(result.current.value).toBe(9));
    await act(async () =>
      old.resolve({
        revision: native.revision,
        values: [{ kind: "declaredCredits", value: 2 }],
      }),
    );
    expect(result.current.value).toBe(9);
  });
  it("rejects a partial batch instead of silently associating the wrong result", async () => {
    native.scheduleQuery.mockResolvedValue({
      revision: native.revision,
      values: [],
    });
    await expect(
      scheduleQueries([{ kind: "declaredCredits", term: "2026-3Q" }]),
    ).rejects.toThrow("invalid schedule snapshot");
  });
  it("rejects misordered result kinds before presenting a batch", async () => {
    native.scheduleQuery.mockResolvedValue({
      revision: native.revision,
      values: [{ kind: "quarterRange", value: 6 }],
    });
    await expect(
      scheduleQueries([{ kind: "declaredCredits", term: "2026-3Q" }]),
    ).rejects.toThrow("invalid schedule snapshot");
  });
  it("failed projections can be retried by a later consumer", async () => {
    native.scheduleQuery.mockRejectedValueOnce(new Error("temporary failure"));
    const first = renderHook(() =>
      useScheduleQuery<number>({ kind: "declaredCredits", term: "2026-3Q" }),
    );
    await waitFor(() =>
      expect(first.result.current.error).toBe("temporary failure"),
    );
    first.unmount();
    native.scheduleQuery.mockResolvedValue({
      revision: native.revision,
      values: [{ kind: "declaredCredits", value: 4 }],
    });
    const next = renderHook(() =>
      useScheduleQuery<number>({ kind: "declaredCredits", term: "2026-3Q" }),
    );
    await waitFor(() => expect(next.result.current.value).toBe(4));
  });
  it("retries a transient failure without a new revision or remount", async () => {
    native.scheduleQuery.mockRejectedValueOnce(new Error("temporary failure"));
    const { result } = renderHook(() =>
      useScheduleQuery<number>({ kind: "declaredCredits", term: "2026-3Q" }),
    );
    await waitFor(() => expect(result.current.error).toBe("temporary failure"));
    native.scheduleQuery.mockResolvedValue({
      revision: native.revision,
      values: [{ kind: "declaredCredits", value: 4 }],
    });
    act(() => result.current.retry?.());
    await waitFor(() => expect(result.current.value).toBe(4));
    expect(result.current.error).toBeUndefined();
  });
  it("ignores old finder responses after the query changes", async () => {
    const old = deferred<unknown>();
    native.finderQuery
      .mockReturnValueOnce(old.promise)
      .mockResolvedValue({ live: "new" });
    const { result, rerender } = renderHook(
      ({ input }) =>
        useFinderQuery({ year: 2026, term: "2026-3Q", keys: [], input }),
      { initialProps: { input: "old" } },
    );
    rerender({ input: "new" });
    await waitFor(() => expect(result.current.value?.live).toBe("new"));
    await act(async () => old.resolve({ live: "old" }));
    expect(result.current.value?.live).toBe("new");
  });
});
