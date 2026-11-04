import assert from "node:assert/strict";
import test from "node:test";

import { markdownToHtml } from "../src/utils/markdown.ts";

test("renders KBOX image references for asynchronous resolution", () => {
  const html = markdownToHtml("![截图](kbox-image://obj_123)");
  assert.match(html, /data-kbox-image="obj_123"/);
  assert.match(html, /alt="截图"/);
});

test("rejects unsafe image protocols and attribute injection", () => {
  const javascript = markdownToHtml("![x](javascript:alert(1))");
  const injected = markdownToHtml('![x](https://example.com/a\" onerror=\"alert(1))');

  assert.doesNotMatch(javascript, /<img/);
  assert.doesNotMatch(injected, /"\s+onerror=/i);
  assert.match(injected, /&quot;/);
});

test("renders Office-style extracted headings and safe links", () => {
  const html = markdownToHtml("# 项目说明\n\n[官网](https://example.com)\n\n[危险](file:///tmp/a)");
  assert.match(html, /<h1>项目说明<\/h1>/);
  assert.match(html, /href="https:\/\/example.com"/);
  assert.match(html, />危险<\/a>/);
  assert.match(html, /href="#"/);
});

test("renders rich-editor inline formatting", () => {
  const html = markdownToHtml("**粗体** *斜体* ~~删除线~~ `code`");
  assert.match(html, /<strong>粗体<\/strong>/);
  assert.match(html, /<em>斜体<\/em>/);
  assert.match(html, /<s>删除线<\/s>/);
  assert.match(html, /<code>code<\/code>/);
});
