import { run, iphoneSimulator } from "./common.mjs";
// Starts booting the smoke run's simulator and returns; the first boot of a
// fresh runner's device takes a minute or two, which CI overlaps with the
// Rust build. ios-smoke.mjs waits for the boot to finish.
const device = iphoneSimulator();
if (device.state !== "Booted") run("xcrun", ["simctl", "boot", device.udid]);
