import { run, generate } from "./common.mjs";
run("cargo", [
  "build",
  "-p",
  "isct-ffi",
  "--features",
  process.argv.includes("--ocr") ? "bindgen,ocr" : "bindgen",
  "--locked",
  "-j",
  "2",
]);
generate();
