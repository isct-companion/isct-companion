/** Disposable validated data; authoritative workspace data belongs in storage/store. */
export interface CacheStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}
export const browserCacheStorage: CacheStorage = {
  getItem: (key) => globalThis.localStorage.getItem(key),
  setItem: (key, value) => globalThis.localStorage.setItem(key, value),
  removeItem: (key) => globalThis.localStorage.removeItem(key),
};
type Entry<T> = { key: string; value: T; updatedAt: number };
export function createCache<T>(options: {
  namespace: string;
  version: number;
  ttlMs: number;
  maxEntries: number;
  maxChars: number;
  validate: (value: unknown) => value is T;
  storage?: CacheStorage;
  now?: () => number;
}) {
  const storageKey = `isct-companion:cache:${options.namespace}`;
  const now = options.now ?? Date.now;
  let memory: Entry<T>[] = [];
  let storageDirty = false;
  let generation = 0;
  const valid = (value: unknown): value is T => {
    try {
      return options.validate(value);
    } catch {
      return false;
    }
  };
  const pending = new Map<string, Promise<T>>();
  function entries(): Entry<T>[] {
    if (!options.storage || storageDirty) return memory;
    try {
      const raw = options.storage.getItem(storageKey);
      if (!raw) return (memory = []);
      if (raw.length > options.maxChars) return [];
      const saved = JSON.parse(raw);
      if (saved.version !== options.version || !Array.isArray(saved.entries))
        return [];
      return (memory = saved.entries
        .filter(
          (e: Entry<T>) =>
            e &&
            typeof e.key === "string" &&
            e.key.length <= 4000 &&
            Number.isFinite(e.updatedAt) &&
            valid(e.value),
        )
        .slice(0, options.maxEntries));
    } catch {
      return memory;
    }
  }
  const encode = (values: Entry<T>[]) =>
    JSON.stringify({ version: options.version, entries: values });
  function set(key: string, value: T, updatedAt = now()): boolean {
    if (key.length > 4000 || !Number.isFinite(updatedAt) || !valid(value))
      return false;
    const next = [
      { key, value, updatedAt },
      ...entries().filter((e) => e.key !== key),
    ].slice(0, options.maxEntries);
    try {
      while (next.length && encode(next).length > options.maxChars) next.pop();
    } catch {
      return false;
    }
    if (!next.length) return false;
    memory = next;
    try {
      options.storage?.setItem(storageKey, encode(next));
      storageDirty = false;
    } catch {
      storageDirty = true; // Preserve newer memory until a write succeeds.
    }
    return true;
  }
  function read(key: string) {
    const entry = entries().find((e) => e.key === key);
    return (
      entry && { ...entry, stale: now() - entry.updatedAt >= options.ttlMs }
    );
  }
  function fetch(
    key: string,
    loader: () => Promise<T>,
    force = false,
  ): Promise<T> {
    const running = pending.get(key);
    if (running) return running;
    const cached = read(key);
    if (cached && !cached.stale && !force) return Promise.resolve(cached.value);
    const started = generation;
    const request = Promise.resolve()
      .then(loader)
      .then((value) => {
        if (started === generation && !set(key, value))
          throw new Error("Invalid or oversized cache response");
        return value;
      })
      .finally(() => {
        if (pending.get(key) === request) pending.delete(key);
      });
    pending.set(key, request);
    return request;
  }
  return {
    read,
    set,
    fetch,
    entries,
    clear() {
      memory = [];
      generation++;
      pending.clear();
      storageDirty = false;
      try {
        options.storage?.removeItem(storageKey);
      } catch {
        storageDirty = true;
      }
    },
  };
}
