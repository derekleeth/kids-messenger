import { describe, expect, it } from "vitest";
import { firstLink, isBigEmoji, normalize, parseJoinLink, site, splitLinks } from "./links";

describe("links", () => {
  it("normalizes only web links", () => {
    expect(normalize("https://example.com/a")).toBe("https://example.com/a");
    expect(normalize("www.lego.com")).toBe("https://www.lego.com");
    expect(normalize("https://example.com).")).toBe("https://example.com");
    expect(normalize("javascript:alert(1)")).toBeNull();
    expect(normalize("file:///C:/x")).toBeNull();
    expect(normalize("http://localhost")).toBeNull();
  });

  it("splits text without losing characters", () => {
    const text = "look (https://scratch.mit.edu/projects/1)! and www.nasa.gov.";
    const parts = splitLinks(text);
    expect(parts.map((p) => p.text).join("")).toBe(text);
    expect(parts.filter((p) => p.url).map((p) => p.url)).toEqual([
      "https://scratch.mit.edu/projects/1",
      "https://www.nasa.gov",
    ]);
    expect(firstLink("no links here")).toBeNull();
  });

  it("names the site", () => {
    expect(site("https://www.minecraft.net/en-us")).toBe("minecraft.net");
  });

  it("spots emoji-only messages", () => {
    expect(isBigEmoji("😂")).toBe(true);
    expect(isBigEmoji("👍🏽 🎉")).toBe(true);
    expect(isBigEmoji("👨‍👩‍👧")).toBe(true);
    expect(isBigEmoji("ok 👍")).toBe(false);
    expect(isBigEmoji("😀😀😀😀")).toBe(false);
  });

  it("reads a join link from a QR code", () => {
    expect(parseJoinLink("kitchat://join?hub=http%3A%2F%2F192.168.1.20%3A8787&code=K7P3QX")).toEqual({
      hub: "http://192.168.1.20:8787",
      code: "K7P3QX",
    });
    expect(parseJoinLink("https://evil.example/?hub=x&code=y")).toBeNull();
    expect(parseJoinLink("kitchat://join?hub=x")).toBeNull();
  });
});
