// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  useFinderEditQueue,
  useFinderQuery,
  type FinderEditQueue,
  type FinderView,
} from "./finder-query";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const native = vi.hoisted(() => ({ finderQuery: vi.fn() }));
vi.mock("./desktop-bridge", () => ({ getDesktopBridge: () => native }));
vi.mock("../store/useWorkspace", () => ({
  useWorkspace: (select: (state: { contentRevision: string }) => unknown) =>
    select({ contentRevision: "fixture" }),
}));

const roots: ReturnType<typeof createRoot>[] = [];
afterEach(() => roots.splice(0).forEach((root) => act(() => root.unmount())));
beforeEach(() => vi.resetAllMocks());

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const view = (keys: string[]) => ({ keys }) as FinderView;

function renderQueue(initial: { year: number; term: string; keys: string[] }) {
  const host = document.createElement("div");
  const root = createRoot(host);
  roots.push(root);
  const setKeys = vi.fn();
  const result = { current: undefined as unknown as FinderEditQueue };
  let props = initial;
  function Probe() {
    result.current = useFinderEditQueue({ ...props, setKeys });
    return null;
  }
  const rerender = (next = props) => {
    props = next;
    act(() => root.render(<Probe />));
  };
  rerender();
  return { result, rerender, setKeys };
}

describe("finder edit ordering", () => {
  it("feeds each rapid action the keys returned by the preceding action", async () => {
    const first = deferred<FinderView>();
    native.finderQuery
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce(view(["first", "second"]));
    const queue = renderQueue({ year: 2026, term: "2026-3Q", keys: [] });

    const one = queue.result.current.run({ kind: "commit", key: "first" });
    const two = queue.result.current.run({ kind: "commit", key: "second" });
    await vi.waitFor(() => expect(native.finderQuery).toHaveBeenCalledOnce());
    first.resolve(view(["first"]));
    await one;
    await two;

    expect(native.finderQuery.mock.calls[1][0]).toMatchObject({
      keys: ["first"],
      action: { kind: "commit", key: "second" },
    });
    expect(queue.setKeys.mock.calls.map(([keys]) => keys)).toEqual([
      ["first"],
      ["first", "second"],
    ]);
  });

  it("does not apply an old session after the term changes", async () => {
    const old = deferred<FinderView>();
    native.finderQuery.mockReturnValueOnce(old.promise);
    const queue = renderQueue({
      year: 2026,
      term: "2026-2Q",
      keys: ["old"],
    });
    const pending = queue.result.current.run({ kind: "commit", key: "late" });
    await vi.waitFor(() => expect(native.finderQuery).toHaveBeenCalledOnce());

    queue.rerender({ year: 2026, term: "2026-3Q", keys: ["new"] });
    old.resolve(view(["old", "late"]));
    await expect(pending).resolves.toBeUndefined();
    expect(queue.setKeys).not.toHaveBeenCalled();

    native.finderQuery.mockResolvedValueOnce(view(["new", "current"]));
    await queue.result.current.run({ kind: "commit", key: "current" });
    expect(native.finderQuery.mock.calls[1][0]).toMatchObject({
      term: "2026-3Q",
      keys: ["new"],
    });
    expect(queue.setKeys).toHaveBeenLastCalledWith(["new", "current"]);
  });
});

describe("finder read cache", () => {
  it("shares one identical Core request between mounted consumers", async () => {
    native.finderQuery.mockResolvedValue(view([]));
    const host = document.createElement("div");
    const root = createRoot(host);
    roots.push(root);
    const result = {
      current: [] as ReturnType<typeof useFinderQuery>[],
    };
    function Probe() {
      const request = { year: 2026, term: "2026-4Q", keys: ["shared"] };
      result.current = [useFinderQuery(request), useFinderQuery(request)];
      return null;
    }
    act(() => root.render(<Probe />));
    await vi.waitFor(() =>
      expect(result.current.map((state) => state.value?.keys)).toEqual([
        [],
        [],
      ]),
    );
    expect(native.finderQuery).toHaveBeenCalledOnce();
  });
});
