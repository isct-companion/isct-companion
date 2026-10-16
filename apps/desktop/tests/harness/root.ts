import { readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * Absolute path of the repository root, for suites that read committed files.
 *
 * `tests/vitest.config.ts` sets `ISCT_REPO_ROOT`; `import.meta.url` is not a
 * file URL inside the jsdom environment, so it cannot be derived from here.
 */
export const repositoryRoot = process.env.ISCT_REPO_ROOT ?? process.cwd();

/** Reads a committed file by repository-relative path. */
export function readRepoFile(path: string): string {
  return readFileSync(join(repositoryRoot, path), "utf8");
}
