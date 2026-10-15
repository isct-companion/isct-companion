import { run, generate, sdkFeatureArgs } from "./common.mjs";
const ocr = process.argv.includes("--ocr");
run("cargo", [
  "build",
  "-p",
  "isct-ffi",
  ...sdkFeatureArgs({ bindgen: true, ocr }),
  "--locked",
  "-j",
  "2",
]);
generate();
