import { describe, expect, it } from "vitest";
import {
  googleDriveClientId,
  googleRedirectScheme,
  iosInfoPlist,
  nativeOAuthEnv,
  oauthClientTarget,
} from "./oauth-config.mjs";

const clients = {
  "google-drive": {
    desktop: "1-desk.apps.googleusercontent.com",
    ios: "1-ios.apps.googleusercontent.com",
  },
};

describe("build-time OAuth config", () => {
  it("picks the client type from Tauri's target platform", () => {
    expect(oauthClientTarget("ios")).toBe("ios");
    expect(oauthClientTarget(undefined, { PLATFORM_NAME: "iphoneos" })).toBe(
      "ios",
    );
    expect(
      oauthClientTarget(undefined, { SDK_NAME: "iphonesimulator27.0" }),
    ).toBe("ios");
    expect(oauthClientTarget("android")).toBe("android");
    expect(oauthClientTarget("darwin")).toBe("desktop");
    expect(oauthClientTarget(undefined)).toBe("desktop");
    expect(googleDriveClientId("ios", clients)).toBe(
      "1-ios.apps.googleusercontent.com",
    );
    expect(googleDriveClientId("android", clients)).toBe("");
  });

  it("derives the iOS callback scheme into the merged Info.plist", () => {
    expect(googleRedirectScheme("1-ios.apps.googleusercontent.com")).toBe(
      "com.googleusercontent.apps.1-ios",
    );
    expect(iosInfoPlist("1-ios.apps.googleusercontent.com")).toContain(
      "<string>com.googleusercontent.apps.1-ios</string>",
    );
    expect(iosInfoPlist("1-ios.apps.googleusercontent.com")).toContain(
      "<key>GIDClientID</key>",
    );
  });

  it("hands the desktop client and its secret to the native build", () => {
    const withSecret = {
      "google-drive": { ...clients["google-drive"], "desktop-secret": "s" },
    };
    expect(nativeOAuthEnv(withSecret, {})).toEqual({
      ISCT_GOOGLE_IOS_CLIENT_ID: "1-ios.apps.googleusercontent.com",
      ISCT_GOOGLE_ANDROID_CLIENT_ID: "",
      ISCT_GOOGLE_DESKTOP_CLIENT_ID: "1-desk.apps.googleusercontent.com",
      ISCT_GOOGLE_DESKTOP_CLIENT_SECRET: "s",
    });
    expect(
      nativeOAuthEnv(withSecret, { ISCT_GOOGLE_DESKTOP_CLIENT_SECRET: "ci" })
        .ISCT_GOOGLE_DESKTOP_CLIENT_SECRET,
    ).toBe("ci");
  });
});
