// Run: node scripts/performance/lms-content.mjs
// Parser microbenchmark only: does not measure native WebView or PDF memory.
import { JSDOM } from "jsdom";
import { performance } from "node:perf_hooks";
import { createServer } from "vite";
// Use the app's module resolver for its extensionless TypeScript imports.
const loader = await createServer({
  configFile: false,
  optimizeDeps: { noDiscovery: true, include: [] },
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
});
let safeContent, textPreview;
try {
  ({ safeContent, textPreview } = await loader.ssrLoadModule(
    "/src/features/lms/content.ts",
  ));
} finally {
  await loader.close();
}
const dom = new JSDOM("");
for (const name of ["document", "DOMParser", "Node", "Element"])
  globalThis[name] = dom.window[name];
const unit =
  '<p>授業計画：NPDA<br>Course schedule: NPDA</p><ul><li>Read the notes before class.</li></ul><a href="https://lms.s.isct.ac.jp/2025/">Lecture PDF</a>';
function measure(run) {
  run();
  const samples = [];
  for (let i = 0; i < 7; i++) {
    const start = performance.now();
    run();
    samples.push(performance.now() - start);
  }
  return Number(samples.sort((a, b) => a - b)[3].toFixed(2));
}
for (const [sections, repeat] of [
  [14, 20],
  [52, 50],
]) {
  const bodies = Array.from({ length: sections }, () => unit.repeat(repeat));
  const oldMs = measure(() =>
    bodies.map((html) => textPreview(safeContent(html))),
  );
  const newMs = measure(() => bodies.map((html) => textPreview(html)));
  console.log(
    JSON.stringify({
      sections,
      charsPerSection: bodies[0].length,
      previousMedianMs: oldMs,
      optimizedMedianMs: newMs,
      reductionPercent: Number((100 * (1 - newMs / oldMs)).toFixed(1)),
    }),
  );
}
dom.window.close();
