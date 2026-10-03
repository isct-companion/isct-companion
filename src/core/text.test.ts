import { describe, expect, it } from "vitest";
import { decodeEntities } from "./text";

describe("decodeEntities", () => {
  it("undoes one level of escaping and leaves plain text alone", () => {
    expect(decodeEntities("Archaeology &amp; Biological Anthropology C")).toBe("Archaeology & Biological Anthropology C");
    expect(decodeEntities("&lt;b&gt; &#39;x&#39; &#x41;")).toBe("<b> 'x' A");
    expect(decodeEntities("R&D & more &amp;amp; &unknown;")).toBe("R&D & more &amp; &unknown;");
  });
});
