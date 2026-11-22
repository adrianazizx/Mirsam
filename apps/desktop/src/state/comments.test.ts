import { describe, expect, it } from "vitest";
import type { DocumentComment } from "../backend";
import { authorsOf, DEFAULT_COMMENT_FILTER, filterComments, sortComments } from "./comments";

const comment = (patch: Partial<DocumentComment>): DocumentComment => ({
  page: 0,
  index: 0,
  kind: "highlight",
  author: null,
  modified: null,
  contents: null,
  markedText: null,
  rect: { x: 10, y: 10, width: 50, height: 12 },
  resolved: false,
  replies: [],
  ...patch,
});

const list = [
  comment({ page: 2, index: 0, kind: "note", author: "Zaid", modified: 100, resolved: true }),
  comment({
    page: 0,
    index: 1,
    author: "سارة",
    modified: 300,
    rect: { x: 0, y: 400, width: 9, height: 9 },
  }),
  comment({
    page: 0,
    index: 2,
    kind: "ink",
    modified: 200,
    rect: { x: 0, y: 50, width: 9, height: 9 },
  }),
  comment({
    page: 1,
    index: 0,
    kind: "textBox",
    author: "Adam",
    modified: 50,
    replies: [{ index: 3, author: "Zaid", modified: 500, contents: "ok" }],
  }),
];
const key = (items: DocumentComment[]) => items.map((item) => `${item.page}:${item.index}`);
const collator = new Intl.Collator("en");

describe("comment filters", () => {
  it("keep comments by status, type, and author", () => {
    expect(key(filterComments(list, DEFAULT_COMMENT_FILTER))).toHaveLength(4);
    expect(key(filterComments(list, { ...DEFAULT_COMMENT_FILTER, status: "resolved" }))).toEqual([
      "2:0",
    ]);
    expect(key(filterComments(list, { ...DEFAULT_COMMENT_FILTER, status: "open" }))).toHaveLength(
      3,
    );
    expect(key(filterComments(list, { ...DEFAULT_COMMENT_FILTER, kind: "markup" }))).toEqual([
      "0:1",
    ]);
    expect(key(filterComments(list, { ...DEFAULT_COMMENT_FILTER, kind: "ink" }))).toEqual(["0:2"]);
    expect(key(filterComments(list, { ...DEFAULT_COMMENT_FILTER, author: "سارة" }))).toEqual([
      "0:1",
    ]);
    // "" is the comments without a name.
    expect(key(filterComments(list, { ...DEFAULT_COMMENT_FILTER, author: "" }))).toEqual(["0:2"]);
  });

  it("sort by place on the page, by latest activity, or by author", () => {
    // Top to bottom within a page, whatever the order of /Annots.
    expect(key(sortComments(list, "page", collator))).toEqual(["0:2", "0:1", "1:0", "2:0"]);
    // A reply is activity: the text box's thread is the newest.
    expect(key(sortComments(list, "newest", collator))).toEqual(["1:0", "0:1", "0:2", "2:0"]);
    // Unnamed last.
    expect(key(sortComments(list, "author", collator))).toEqual(["1:0", "2:0", "0:1", "0:2"]);
  });

  it("list each author once, in the UI language's order, with no name last", () => {
    expect(authorsOf(list, collator)).toEqual(["Adam", "Zaid", "سارة", ""]);
    expect(authorsOf(list.slice(0, 1), collator)).toEqual(["Zaid"]);
  });
});
