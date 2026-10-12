// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MantineProvider } from "@mantine/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appTheme } from "../../app/theme";

const mocks = vi.hoisted(() => {
  const order: string[] = [];
  return {
    order,
    recognize: vi.fn(),
    loginListeners: [] as Array<(event: unknown) => void>,
    sessionListeners: [] as Array<(event: unknown) => void>,
    bridge: {
      schoolCredentialStatus: vi.fn(async () => ({
        science: {
          saved: true,
          username: "saved-id",
          hasOtp: true,
          hasMatrix: false,
        },
        tokyoTech: {
          saved: false,
          username: "",
          hasOtp: false,
          hasMatrix: false,
        },
      })),
      saveSchoolCredentials: vi.fn(async () => {
        order.push("save");
        return true;
      }),
      testSchoolConnection: vi.fn(async () => {
        order.push("login");
        return {
          site: "science-portal",
          url: "https://example.test",
          phase: "login",
        };
      }),
      deleteSchoolCredentials: vi.fn(async () => true),
      copySchoolMatrix: vi.fn(async () => true),
      onSchoolLoginStatus: vi.fn((callback: (event: unknown) => void) => {
        mocks.loginListeners.push(callback);
        return vi.fn();
      }),
      onSchoolSession: vi.fn((callback: (event: unknown) => void) => {
        mocks.sessionListeners.push(callback);
        return vi.fn();
      }),
    },
  };
});

vi.mock("../../core/matrix-ocr", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../core/matrix-ocr")>()),
  recognizeMatrixImage: mocks.recognize,
}));

vi.mock("../../app/native", () => ({ desktopBridge: mocks.bridge }));

import { schoolSessions } from "../../platform/school-session";
import { SchoolAccounts } from "./SchoolAccounts";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
const notify = vi.fn();
const showLoginPage = vi.fn();

const flush = async () => {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
};

function button(scope: ParentNode, name: string) {
  const match = [...scope.querySelectorAll("button")].find(
    (item) => item.textContent?.trim() === name,
  );
  if (!match) throw new Error(`Missing ${name} button`);
  return match as HTMLButtonElement;
}

async function mount(matrixOcrAvailable = true) {
  root = createRoot(host);
  await act(async () => {
    root.render(
      <MantineProvider theme={appTheme} defaultColorScheme="light">
        <SchoolAccounts
          notify={notify}
          showLoginPage={showLoginPage}
          matrixOcrAvailable={matrixOcrAvailable}
        />
      </MantineProvider>,
    );
  });
  await flush();
}

beforeEach(async () => {
  vi.clearAllMocks();
  mocks.order.length = 0;
  mocks.loginListeners.length = 0;
  mocks.sessionListeners.length = 0;
  schoolSessions.reset();
  host = document.createElement("div");
  document.body.append(host);
  await mount();
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
});

describe("school account cards", () => {
  it("shows Connect, Forget, and the session badge once it is known", () => {
    const science = host.querySelectorAll("section")[0];
    expect(button(science, "Connect")).toBeTruthy();
    expect(button(science, "Forget")).toBeTruthy();
    for (const actions of host.querySelectorAll(".schoolAccountActions")) {
      expect(
        [...actions.querySelectorAll("button")].map((button) =>
          button.textContent?.trim(),
        ),
      ).not.toContain("Save");
    }
    expect(
      science.querySelectorAll(".schoolAccountActions button"),
    ).toHaveLength(2);
    // Saved but not yet signed in this launch: no badge.
    expect(science.querySelector(".mantine-Badge-root")).toBeNull();
    const tokyoTech = host.querySelectorAll("section")[1];
    expect(tokyoTech.textContent).toContain("Not set up");
    expect(
      [...tokyoTech.querySelectorAll("button")].map((x) => x.textContent),
    ).not.toContain("Forget");
  });

  it("Connect saves credentials before starting the tested login flow", async () => {
    const science = host.querySelectorAll("section")[0];
    await act(async () => button(science, "Connect").click());
    await flush();
    expect(mocks.order).toEqual(["save", "login"]);
    expect(mocks.bridge.saveSchoolCredentials).toHaveBeenCalledWith(
      "science",
      expect.objectContaining({ username: "saved-id" }),
    );
    expect(mocks.bridge.testSchoolConnection).toHaveBeenCalledWith(
      "science",
      expect.objectContaining({ username: "saved-id" }),
    );
  });

  it("renders the fixed ID → Password → Code → Signed in progress labels", async () => {
    const science = host.querySelectorAll("section")[0];
    await act(async () => button(science, "Connect").click());
    expect(science.textContent).toContain("ID");
    expect(science.textContent).toContain("Password");
    expect(science.textContent).toContain("Code");
    expect(science.textContent).toContain("Signed in");

    await act(async () => {
      mocks.loginListeners.forEach((listener) =>
        listener({ profile: "science", state: "progress", message: "otp" }),
      );
    });
    expect(
      science.querySelector(".schoolConnection")?.getAttribute("data-state"),
    ).toBe("progress");
  });

  it("updates the session badge from native session events", async () => {
    const science = host.querySelectorAll("section")[0];
    await act(async () => {
      mocks.sessionListeners.forEach((listener) =>
        listener({ profile: "science", state: "active" }),
      );
    });
    expect(science.textContent).toContain("Signed in");
    await act(async () => {
      mocks.sessionListeners.forEach((listener) =>
        listener({ profile: "science", state: "failed" }),
      );
    });
    expect(science.textContent).toContain("Sign-in failed");
  });

  it("Forget confirms, deletes the selected profile, and reports completion", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const science = host.querySelectorAll("section")[0];
    await act(async () => button(science, "Forget").click());
    await flush();
    expect(window.confirm).toHaveBeenCalledWith(
      "Forget saved Science Tokyo Portal credentials on this device?",
    );
    expect(mocks.bridge.deleteSchoolCredentials).toHaveBeenCalledWith(
      "science",
    );
    expect(notify).toHaveBeenCalledWith(
      "Science Tokyo Portal account forgotten.",
    );
  });

  it("opens the login page only for a user-required event from this card", async () => {
    const science = host.querySelectorAll("section")[0];
    await act(async () => button(science, "Connect").click());
    await act(async () => {
      mocks.loginListeners.forEach((listener) =>
        listener({
          profile: "science",
          state: "needs-user",
          message: "Complete sign in",
        }),
      );
    });
    expect(showLoginPage).toHaveBeenCalledWith("science");
  });
});

async function scanMatrix(results: { text: string; confidence: number }[]) {
  mocks.recognize.mockResolvedValueOnce(results);
  const file = host.querySelector('input[type="file"]')!;
  Object.defineProperty(file, "files", {
    configurable: true,
    value: [new File(["test"], "card.png", { type: "image/png" })],
  });
  await act(async () =>
    file.dispatchEvent(new Event("change", { bubbles: true })),
  );
  await flush();
}

describe("matrix review and save", () => {
  it("hides image scanning when OCR is unavailable on this target", async () => {
    act(() => root.unmount());
    await mount(false);
    expect(
      [...host.querySelectorAll("button")].map((item) =>
        item.textContent?.trim(),
      ),
    ).not.toContain("Scan image");
  });

  it("shows three confidence bands in plaintext, then clears and masks after Connect saves", async () => {
    const result = Array.from({ length: 70 }, (_, i) => ({
      text: "A",
      confidence: i === 0 ? 0.96 : i === 1 ? 0.82 : 0.7,
    }));
    await scanMatrix(result);
    const card = host.querySelectorAll("section")[1];
    const inputs = [
      ...card.querySelectorAll<HTMLInputElement>(".matrixGrid input"),
    ];
    expect(inputs[0].value).toBe("A");
    expect(inputs.slice(0, 3).map((input) => input.dataset.confidence)).toEqual(
      ["high", "medium", "low"],
    );
    mocks.bridge.schoolCredentialStatus.mockResolvedValueOnce({
      science: {
        saved: true,
        username: "saved-id",
        hasOtp: true,
        hasMatrix: false,
      },
      tokyoTech: {
        saved: true,
        username: "id",
        hasOtp: false,
        hasMatrix: true,
      },
    });
    await act(async () => button(card, "Connect").click());
    await flush();
    expect(mocks.bridge.saveSchoolCredentials).toHaveBeenCalledWith(
      "tokyo-tech",
      expect.objectContaining({ matrix: "A".repeat(70) }),
    );
    expect(mocks.bridge.testSchoolConnection).toHaveBeenCalled();
    expect(
      inputs.every(
        (input) =>
          input.value === "" &&
          input.placeholder === "•" &&
          !input.dataset.confidence,
      ),
    ).toBe(true);
    await act(async () => inputs[0].focus());
    expect(inputs[0].placeholder).toBe("•");
  });
  it("does not save incomplete scans or discard the readable cells", async () => {
    await scanMatrix(
      Array.from({ length: 70 }, (_, i) => ({
        text: i === 0 ? "L" : "",
        confidence: i === 0 ? 0.92 : 0,
      })),
    );
    const card = host.querySelectorAll("section")[1];
    await act(async () => button(card, "Connect").click());
    expect(mocks.bridge.saveSchoolCredentials).not.toHaveBeenCalled();
    expect(
      card.querySelector<HTMLInputElement>(".matrixGrid input")!.value,
    ).toBe("L");
    expect(
      card.querySelectorAll('[data-confidence="low"]').length,
    ).toBeGreaterThanOrEqual(69);
  });
  it("keeps plaintext and confidence when saving fails", async () => {
    await scanMatrix(
      Array.from({ length: 70 }, () => ({ text: "C", confidence: 0.94 })),
    );
    mocks.bridge.saveSchoolCredentials.mockRejectedValueOnce(
      new Error("Vault unavailable"),
    );
    const card = host.querySelectorAll("section")[1];
    await act(async () => button(card, "Connect").click());
    expect(
      card.querySelector<HTMLInputElement>(".matrixGrid input")!.value,
    ).toBe("C");
    expect(card.textContent).toContain("Vault unavailable");
  });
});

describe("repeat matrix scans", () => {
  it.each(["success", "empty", "error"])(
    "resets the file input after %s so the same photo can be selected again",
    async (outcome) => {
      const input = host.querySelector<HTMLInputElement>('input[type="file"]')!;
      let selected = "C:\\fakepath\\card.png";
      Object.defineProperty(input, "value", {
        configurable: true,
        get: () => selected,
        set: (value) => {
          selected = value;
        },
      });
      const image = new File(["test"], "card.png", { type: "image/png" });
      Object.defineProperty(input, "files", {
        configurable: true,
        value: [image],
      });
      const result = Array.from({ length: 70 }, () => ({
        text: outcome === "empty" ? "" : "A",
        confidence: 0.95,
      }));
      if (outcome === "error")
        mocks.recognize.mockRejectedValueOnce(new Error("Scan failed"));
      else mocks.recognize.mockResolvedValueOnce(result);
      await act(async () =>
        input.dispatchEvent(new Event("change", { bubbles: true })),
      );
      await flush();
      expect(selected).toBe("");
      expect(button(host, "Scan image").disabled).toBe(false);
      // A browser dispatches change again for the same file only after this reset.
      selected = "C:\\fakepath\\card.png";
      mocks.recognize.mockResolvedValueOnce(
        Array.from({ length: 70 }, () => ({ text: "B", confidence: 0.95 })),
      );
      await act(async () =>
        input.dispatchEvent(new Event("change", { bubbles: true })),
      );
      await flush();
      expect(mocks.recognize).toHaveBeenCalledTimes(2);
      expect(selected).toBe("");
      expect(
        host.querySelector<HTMLInputElement>(".matrixGrid input")!.value,
      ).toBe("B");
    },
  );
});

// Cell i follows the grid's visual row order: A1…J1, A2…J2, …
const matrixLetters = Array.from({ length: 70 }, (_, i) =>
  String.fromCharCode(65 + (i % 26)),
);

const matrixInputs = () => [
  ...host
    .querySelectorAll("section")[1]
    .querySelectorAll<HTMLInputElement>(".matrixGrid input"),
];

async function pasteMatrix(text: string) {
  const paste = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(paste, "clipboardData", {
    value: { getData: () => text },
  });
  // Pasting into a middle cell must not spread the letters from there.
  await act(async () => matrixInputs()[5].dispatchEvent(paste));
}

describe("matrix files", () => {
  it("pastes whitespace-formatted text into the right grid cells", async () => {
    const text = Array.from({ length: 7 }, (_, row) =>
      matrixLetters.slice(row * 10, row * 10 + 10).join(" "),
    ).join("\n");
    await pasteMatrix(text);
    expect(matrixInputs().map((input) => input.value)).toEqual(matrixLetters);
  });

  it("asks before copying the plaintext Matrix", async () => {
    act(() => root.unmount());
    mocks.bridge.schoolCredentialStatus.mockResolvedValueOnce({
      science: { saved: false, username: "", hasOtp: false, hasMatrix: false },
      tokyoTech: {
        saved: true,
        username: "id",
        hasOtp: false,
        hasMatrix: true,
      },
    });
    await mount();
    const card = host.querySelectorAll("section")[1];
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false);
    await act(async () => button(card, "Copy").click());
    expect(mocks.bridge.copySchoolMatrix).not.toHaveBeenCalled();

    confirm.mockReturnValueOnce(true);
    await act(async () => button(card, "Copy").click());
    await flush();
    expect(mocks.bridge.copySchoolMatrix).toHaveBeenCalledOnce();
  });
});
