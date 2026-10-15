import { afterEach, describe, expect, it } from "vitest";
import ja from "../../../../resources/locales/ja.json";
import zh from "../../../../resources/locales/zh.json";
import i18n, { intlLocale, t } from ".";

afterEach(() => {
  void i18n.changeLanguage("en");
});
describe("Chinese UI", () => {
  it("has exactly the Japanese key set and preserves interpolation placeholders", () => {
    expect(Object.keys(zh).sort()).toEqual(Object.keys(ja).sort());
    for (const [key, value] of Object.entries(zh)) {
      expect(value.trim(), key).not.toBe("");
      expect(
        [...value.matchAll(/{{[^}]+}}/g)].map((match) => match[0]).sort(),
        key,
      ).toEqual(
        [...key.matchAll(/{{[^}]+}}/g)].map((match) => match[0]).sort(),
      );
    }
  });
  it("uses Chinese translations and all three date locales", async () => {
    await i18n.changeLanguage("zh");
    expect(t("Home")).toBe("首页");
    expect(t("{{count}} files", { count: 3 })).toBe("3 个文件");
    expect([intlLocale("en"), intlLocale("ja"), intlLocale("zh")]).toEqual([
      "en-GB",
      "ja-JP",
      "zh-CN",
    ]);
  });
});
