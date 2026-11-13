// @vitest-environment jsdom
// At 375 px there is no horizontal scroll because of how the layouts stack: the composer's two columns become one (the kit's
// page-split at 900 px), rows wrap, nothing has a width of its own; the inbox desk is one column under 720 px and its rows clip
// their text. (jsdom has no layout: these hold the classes and rules the stacking rests on.)

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const mine = readFileSync(`${process.cwd()}/src/styles.css`, "utf8");
const kit = readFileSync(`${process.cwd()}/../../ferrum/web/src/styles.css`, "utf8");
const composer = readFileSync(`${process.cwd()}/src/Composer.tsx`, "utf8");

describe("phone width", () => {
  it("the composer is one page-split that the kit stacks, and its rows wrap", () => {
    expect(composer).toContain("<PageSplit");
    expect(kit).toMatch(/@media \(max-width: 900px\)\s*\{\s*\.page-split\s*\{\s*grid-template-columns: minmax\(0, 1fr\)/);
    expect(kit).toMatch(/\.row \{[^}]*flex-wrap: wrap/);
  });
  it("the inbox is one column and its rows clip", () => {
    expect(mine).toMatch(/@media \(max-width: 720px\)\s*\{\s*\.inbox-desk\s*\{\s*grid-template-columns: minmax\(0, 1fr\)/);
    expect(mine).toMatch(/\.inbox-text \{[^}]*min-width: 0/);
    expect(mine).toMatch(/\.inbox-snippet \{[^}]*text-overflow: ellipsis/);
  });
  it("preview media and the media grid fit their column", () => {
    expect(mine).toMatch(/\.preview-media \{[^}]*repeat\(auto-fill, minmax\(/);
    expect(mine).toMatch(/\.media-grid \{[^}]*repeat\(auto-fill, minmax\(/);
    expect(mine).toMatch(/\.preview-text \{[^}]*overflow-wrap: anywhere/);
  });
});
