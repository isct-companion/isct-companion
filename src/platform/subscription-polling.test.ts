import { afterEach, describe, expect, it } from "vitest";
import { seedWorkspace } from "../core/model";
import { subscriptionNetwork, subscriptionYear } from "./subscription-polling";

const onlineDescriptor = Object.getOwnPropertyDescriptor(navigator, "onLine");
const connectionDescriptor = Object.getOwnPropertyDescriptor(
  navigator,
  "connection",
);

function setNetwork(
  online: boolean,
  connection?: { type?: string; saveData?: boolean; metered?: boolean },
) {
  Object.defineProperty(navigator, "onLine", {
    configurable: true,
    value: online,
  });
  Object.defineProperty(navigator, "connection", {
    configurable: true,
    value: connection,
  });
}

function restoreProperty(
  name: "onLine" | "connection",
  descriptor?: PropertyDescriptor,
) {
  if (descriptor) Object.defineProperty(navigator, name, descriptor);
  else Reflect.deleteProperty(navigator, name);
}

afterEach(() => {
  restoreProperty("onLine", onlineDescriptor);
  restoreProperty("connection", connectionDescriptor);
});

describe("reference subscription shell policy", () => {
  it("selects the admission year only for department data", () => {
    const data = seedWorkspace();
    data.selectedTerm = "2026-3Q";
    data.academicProfile.admissionYear = 2024;

    expect(subscriptionYear(data, "departments")).toBe(2024);
    for (const kind of ["calendar", "important-dates", "exams"] as const)
      expect(subscriptionYear(data, kind)).toBe(2026);
  });

  it("pauses offline and metered connections", () => {
    setNetwork(false);
    expect(subscriptionNetwork("native-desktop")).toBe("offline");

    for (const connection of [
      { type: "cellular" },
      { type: "wifi", saveData: true },
      { type: "wifi", metered: true },
    ]) {
      setNetwork(true, connection);
      expect(subscriptionNetwork("native-desktop")).toBe("cellular");
    }
  });

  it("uses the platform default when connection details are unavailable", () => {
    setNetwork(true, { type: "wifi" });
    expect(subscriptionNetwork("native-mobile")).toBe("wifi");
    setNetwork(true);
    expect(subscriptionNetwork("native-mobile")).toBe("cellular");
    expect(subscriptionNetwork("native-desktop")).toBe("wifi");
    expect(subscriptionNetwork("browser-preview")).toBe("wifi");
  });
});
