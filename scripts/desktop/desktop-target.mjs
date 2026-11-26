import { execFileSync } from "node:child_process";

export function hostTriple() {
  const info = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  return /^host: (.+)$/m.exec(info)?.[1];
}

export function desktopTarget(flags) {
  const targetIndex = flags.indexOf("--target");
  return (
    flags.find((flag) => flag.startsWith("--target="))?.slice(9) ??
    (targetIndex < 0 ? undefined : flags[targetIndex + 1]) ??
    process.env.CARGO_BUILD_TARGET ??
    hostTriple()
  );
}

export function hasCampusVpn(target) {
  return [
    "aarch64-apple-darwin",
    "universal-apple-darwin",
    "x86_64-pc-windows-msvc",
  ].includes(target);
}
