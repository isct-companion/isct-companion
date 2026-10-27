import { run, generate, sdkFeatureArgs } from "./common.mjs";
const ocr = process.argv.includes("--ocr");
run("cargo", [
  "build",
  "-p",
  "app-ffi",
  ...sdkFeatureArgs({ bindgen: true, ocr }),
  "--locked",
  "-j",
  "2",
]);
generate();
