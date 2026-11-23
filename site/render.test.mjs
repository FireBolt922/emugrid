import assert from "node:assert/strict";
import { test } from "node:test";
import { classifyAsset, esc, formatSize, prepareReleases, renderPage } from "./render.mjs";

const asset = (name, size = 3_000_000) => ({
  name,
  size,
  browser_download_url: `https://github.com/o/r/releases/download/x/${name}`,
});

const api = [
  {
    tag_name: "v0.2.0",
    name: "emugrid 0.2.0",
    html_url: "https://github.com/o/r/releases/tag/v0.2.0",
    published_at: "2026-11-01T10:00:00Z",
    draft: false,
    prerelease: false,
    assets: [
      asset("emugrid-0.2.0-macos-universal.dmg"),
      asset("emugrid-0.2.0-windows-x86_64.zip"),
      asset("emugrid-0.2.0-linux-x86_64.tar.gz"),
      asset("SHA256SUMS.txt", 300),
    ],
  },
  {
    tag_name: "v0.3.0",
    name: "emugrid 0.3.0",
    html_url: "https://github.com/o/r/releases/tag/v0.3.0",
    published_at: "2026-12-01T10:00:00Z",
    draft: false,
    prerelease: true,
    assets: [asset("emugrid-0.3.0-macos-universal.dmg"), asset("emugrid-0.3.0-linux-x86_64.tar.gz")],
  },
  {
    tag_name: "v0.4.0",
    name: "draft",
    html_url: "https://github.com/o/r/releases/tag/v0.4.0",
    published_at: null,
    draft: true,
    prerelease: false,
    assets: [],
  },
];

test("classifies release files by name", () => {
  assert.equal(classifyAsset("emugrid-1.0.0-macos-universal.dmg"), "macos");
  assert.equal(classifyAsset("emugrid-macos-arm64.zip"), "macos");
  assert.equal(classifyAsset("emugrid-1.0.0-windows-x86_64.zip"), "windows");
  assert.equal(classifyAsset("emugrid-1.0.0-linux-x86_64.tar.gz"), "linux");
  assert.equal(classifyAsset("SHA256SUMS.txt"), "checksums");
  assert.equal(classifyAsset("notes.md"), null);
});

test("formats sizes", () => {
  assert.equal(formatSize(512), "512 B");
  assert.equal(formatSize(2048), "2.0 KB");
  assert.equal(formatSize(3 * 1024 * 1024), "3.0 MB");
});

test("drops drafts and sorts newest first", () => {
  const releases = prepareReleases(api);
  assert.deepEqual(
    releases.map((r) => r.version),
    ["0.3.0", "0.2.0"],
  );
  assert.equal(releases[0].prerelease, true);
  assert.equal(releases[1].downloads.windows.name, "emugrid-0.2.0-windows-x86_64.zip");
});

test("renders the latest release and keeps the history", () => {
  const html = renderPage(prepareReleases(api), { repo: "o/r" });
  assert.match(html, /Version 0\.3\.0/);
  assert.match(html, /Test build/);
  assert.match(html, /emugrid-0\.3\.0-macos-universal\.dmg/);
  assert.match(html, /Not in this release/); // 0.3.0 has no Windows build
  assert.match(html, /id="history"/);
  assert.match(html, /emugrid-0\.2\.0-windows-x86_64\.zip/);
  assert.doesNotMatch(html, /0\.4\.0/);
});

test("renders an empty page before the first release", () => {
  const html = renderPage([], { repo: "o/r" });
  assert.match(html, /No release yet/);
  assert.doesNotMatch(html, /id="history"/);
});

test("escapes values from releases", () => {
  assert.equal(esc(`<a href="x">'&'</a>`), "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;");
  const evil = [{ ...api[0], tag_name: "v<script>", assets: [] }];
  const html = renderPage(prepareReleases(evil), { repo: "o/r" });
  assert.doesNotMatch(html, /<script>/);
});
