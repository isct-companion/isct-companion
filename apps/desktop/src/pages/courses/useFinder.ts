import { useEffect, useReducer, useRef } from "react";

import { desktopBridge } from "../../app/native";
import {
  finderQuery,
  type FinderAction,
  type FinderPage,
  type FinderRowView,
} from "../../platform/finder-query";
import { viewRevision } from "../../platform/view-query";
import { useWorkspace } from "../../store/useWorkspace";

const PAGE = 100;
/** Core's largest page; a refresh keeps at most this many loaded rows. */
const MOST = 1000;

const message = (reason: unknown) =>
  reason instanceof Error ? reason.message : String(reason);

/**
 * The finder's one owner, shared by the timetable (slot highlight, placed
 * codes) and the search panel. One state of (keys, input, unavailable shown)
 * is one Core request: typing is latest-wins, actions (commit, time toggle,
 * paste) run in order so each starts from the keys Core returned for the
 * previous one, and a key replacement drops the actions still in flight.
 */
export type Finder = {
  /** Core's keys for the current conditions. */
  keys: string[];
  /** Conditions committed here and not yet confirmed by Core. */
  pending: string[];
  input: string;
  /** The latest page for the current conditions. */
  page?: FinderPage;
  /** Its rows and the pages loaded after it. */
  rows: FinderRowView[];
  error?: string;
  /** Timetable slots of the time conditions, with local toggles applied. */
  slots: Set<string>;
  placedCodes: Set<string>;
  includeIneligible: boolean;
  /** The request number of the shown page; later requests have larger ones. */
  shown: number;
  /** The number of the latest request sent. */
  issued(): number;
  type(value: string, query?: boolean): void;
  commit(key: string): void;
  toggleTime(key: string, append: boolean, slot?: string): void;
  replace(update: string[] | ((current: string[]) => string[])): void;
  paste(text: string): Promise<FinderPage | undefined>;
  showIneligible(show: boolean): void;
  loadMore(): void;
};

type State = {
  identity: string;
  term: string;
  keys: string[];
  pending: string[];
  input: string;
  includeIneligible: boolean;
  page?: FinderPage;
  rows: FinderRowView[];
  shown: number;
  /** Requests sent; a response that is not the latest is dropped. */
  sent: number;
  /** Bumped by a key replacement or a new session; older actions are void. */
  generation: number;
  actions: number;
  tail: Promise<unknown>;
  /** A search waiting for the actions in flight. */
  searchAfter: boolean;
  /** The input of the last search sent, so a repeat is not sent again. */
  searched?: string;
  loadingMore: boolean;
  error?: string;
  slots: Set<string> | null;
};

export function useFinder({
  year,
  term,
  session,
  enabled,
  defaultKeys,
}: {
  year: number;
  term: string;
  /** A new session (a route visit, a term) starts with no input or page. */
  session: string;
  enabled: boolean;
  /** The conditions a newly opened term starts with. */
  defaultKeys: string[];
}): Finder {
  const [, render] = useReducer((count: number) => count + 1, 0);
  const live = useRef({ year, term, enabled });
  live.current = { year, term, enabled };
  const state = useRef<State>(undefined as unknown as State);
  if (!state.current || state.current.identity !== session) {
    const previous = state.current;
    state.current = {
      identity: session,
      term,
      keys:
        previous && previous.term === term ? previous.keys : [...defaultKeys],
      pending: [],
      input: "",
      includeIneligible: false,
      rows: [],
      shown: previous?.sent ?? 0,
      sent: previous?.sent ?? 0,
      generation: (previous?.generation ?? 0) + 1,
      actions: 0,
      tail: Promise.resolve(),
      searchAfter: false,
      loadingMore: false,
      slots: null,
    };
  }
  const s = state.current;
  const started = useRef<string | null>(null);

  const request = (limit = PAGE) => ({
    year: live.current.year,
    term: live.current.term,
    keys: s.keys,
    input: s.input,
    includeIneligible: s.includeIneligible,
    offset: 0,
    limit,
  });
  const show = (page: FinderPage, sent: number, input: string) => {
    s.page = page;
    s.rows = page.rows;
    s.shown = sent;
    s.error = undefined;
    if (!s.actions) s.slots = null;
    render();
    // A word typed with a trailing space becomes its condition.
    if (page.commitKey && input === s.input && /\s$/.test(input))
      commit(page.commitKey);
  };
  const fail = (reason: unknown) => {
    s.error = message(reason);
    render();
  };
  function search(limit = PAGE) {
    if (!live.current.enabled) return;
    if (s.actions) {
      s.searchAfter = true;
      return;
    }
    const sent = ++s.sent;
    const generation = s.generation;
    const input = s.input;
    s.searched = input;
    render();
    finderQuery(request(limit)).then(
      (page) => {
        if (sent === s.sent && generation === s.generation)
          show(page, sent, input);
      },
      (reason) => {
        if (sent === s.sent && generation === s.generation) fail(reason);
      },
    );
  }
  function act(action: FinderAction): Promise<FinderPage | undefined> {
    const generation = s.generation;
    s.actions += 1;
    render();
    const run = s.tail.then(async () => {
      if (generation !== s.generation) return undefined;
      const sent = ++s.sent;
      const input = s.input;
      s.searched = input;
      const page = await finderQuery({ ...request(), action });
      if (generation !== s.generation) return undefined;
      s.keys = page.keys;
      if (sent === s.sent) show(page, sent, input);
      return page;
    });
    s.tail = run.catch(() => undefined);
    void run
      .catch((reason) => {
        if (generation === s.generation) s.error = message(reason);
      })
      .finally(() => {
        if (generation !== s.generation) return;
        s.actions -= 1;
        if (s.actions) return;
        s.pending = [];
        s.slots = null;
        if (s.searchAfter) {
          s.searchAfter = false;
          search();
        }
        render();
      });
    return run;
  }
  function commit(key: string) {
    s.input = "";
    s.pending = [...s.pending, key];
    act({ kind: "commit", key }).catch(() => undefined);
  }

  // A new session searches once it is enabled; an action may have started it.
  useEffect(() => {
    if (!enabled || started.current === session) return;
    started.current = session;
    if (s.sent === s.shown && !s.actions) search();
  }, [enabled, session]); // eslint-disable-line react-hooks/exhaustive-deps

  // Committed records the finder reads, or a change to the shown year's
  // catalog, refresh the current page with as many rows as are loaded.
  // Core sends a download's commits at most every 250 ms, so its results
  // appear department by department.
  const refresh = useRef(() => {});
  refresh.current = () => {
    if (started.current === session)
      search(Math.min(MOST, Math.max(PAGE, s.rows.length)));
  };
  // What the shown page read besides the catalog, as Core names it.
  const reads = s.page?.reads ?? [];
  const revision = useWorkspace((snapshot) =>
    viewRevision(snapshot.viewRevisions, reads),
  );
  // The first page names its reads; that is not a change to refresh for.
  const readsKey = reads.join(",");
  const refreshed = useRef({ readsKey, revision });
  useEffect(() => {
    const last = refreshed.current;
    refreshed.current = { readsKey, revision };
    if (last.readsKey === readsKey && last.revision !== revision)
      refresh.current();
  }, [readsKey, revision]);
  useEffect(
    () =>
      desktopBridge?.onCatalogChanged?.((changed) => {
        if (changed.year === live.current.year) refresh.current();
      }),
    [],
  );

  const slots = s.slots ?? new Set(s.page?.activeSlots ?? []);
  return {
    keys: s.keys,
    pending: s.pending,
    input: s.input,
    page: s.page,
    rows: s.rows,
    error: s.error,
    slots,
    placedCodes: new Set(s.page?.placedCodes ?? []),
    includeIneligible: s.includeIneligible,
    shown: s.shown,
    issued: () => s.sent,
    type(value, query = true) {
      s.input = value;
      render();
      if (query && value !== s.searched) search();
    },
    commit,
    toggleTime(key, append, slot) {
      if (slot) {
        const next = new Set(slots);
        if (next.has(slot)) next.delete(slot);
        else {
          if (!append) next.clear();
          next.add(slot);
        }
        s.slots = next;
      }
      // A failure is shown as the finder's error.
      act({ kind: "toggleTime", key, append }).catch(() => undefined);
    },
    replace(update) {
      s.keys = typeof update === "function" ? update(s.keys) : update;
      s.generation += 1;
      s.actions = 0;
      s.tail = Promise.resolve();
      s.pending = [];
      s.slots = null;
      s.searchAfter = false;
      search();
      render();
    },
    paste: (text) => act({ kind: "paste", text }),
    showIneligible(show) {
      s.includeIneligible = show;
      search();
    },
    loadMore() {
      const page = s.page;
      if (
        !page ||
        s.loadingMore ||
        s.sent !== s.shown ||
        s.input !== s.searched ||
        s.rows.length >= page.total
      )
        return;
      const sent = s.sent;
      const generation = s.generation;
      s.loadingMore = true;
      void finderQuery({ ...request(), offset: s.rows.length })
        .then(
          (more) => {
            if (sent !== s.sent || generation !== s.generation) return;
            s.rows = [...s.rows, ...more.rows];
          },
          (reason) => {
            if (sent === s.sent) s.error = message(reason);
          },
        )
        .finally(() => {
          s.loadingMore = false;
          render();
        });
    },
  };
}
