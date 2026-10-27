import { afterEach, beforeEach } from "vitest";
import { installDomGlobals, installWindowGlobals, resetDocument } from "./dom";
import { resetQueries } from "@app/platform/query-client";

const scope = globalThis as unknown as Record<string, unknown>;

// jsdom workers only; node suites (parsers, tooling) skip every branch below.
if (typeof globalThis.window !== "undefined") {
  installDomGlobals(globalThis);
  installWindowGlobals(window as unknown as Window & typeof globalThis);
  beforeEach(resetDocument);
  afterEach(resetDocument);
  // Each test starts with no cached Core answers.
  beforeEach(resetQueries);
} else {
  scope.IS_REACT_ACT_ENVIRONMENT = true;
}
