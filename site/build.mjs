// Builds the download page into a folder for GitHub Pages.
//
//   node site/build.mjs --repo OWNER/NAME --out _site [--releases file.json]
//
// Without --releases it reads the published releases from the GitHub API
// (GITHUB_TOKEN is used when set, to avoid the anonymous rate limit). The
// fonts are copied from the app so the page and the app look alike.
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { prepareReleases, renderPage } from "./render.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");

const { values } = parseArgs({
  options: {
    repo: { type: "string" },
    out: { type: "string", default: "_site" },
    releases: { type: "string" },
  },
});
if (!values.repo || !/^[\w.-]+\/[\w.-]+$/.test(values.repo)) {
  console.error("Usage: node site/build.mjs --repo OWNER/NAME [--out DIR] [--releases FILE]");
  process.exit(2);
}

async function fetchReleases(repo) {
  const headers = { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28" };
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const all = [];
  let url = `https://api.github.com/repos/${repo}/releases?per_page=100`;
  while (url) {
    const res = await fetch(url, { headers });
    if (!res.ok) throw new Error(`GET ${url}: ${res.status} ${await res.text()}`);
    all.push(...(await res.json()));
    url = /<([^>]+)>;\s*rel="next"/.exec(res.headers.get("link") ?? "")?.[1];
  }
  return all;
}

const apiReleases = values.releases
  ? JSON.parse(await readFile(values.releases, "utf8"))
  : await fetchReleases(values.repo);
const releases = prepareReleases(apiReleases);

const out = values.out;
const files = [
  ["site/site.css", "site.css"],
  ["site/site.js", "site.js"],
  ["site/icon.svg", "icon.svg"],
  ["assets/fonts/Inter-Regular.ttf", "fonts/Inter-Regular.ttf"],
  ["assets/fonts/Inter-SemiBold.ttf", "fonts/Inter-SemiBold.ttf"],
  ["assets/fonts/JetBrainsMono-Regular.ttf", "fonts/JetBrainsMono-Regular.ttf"],
];
for (const [from, to] of files) {
  await mkdir(dirname(join(out, to)), { recursive: true });
  await copyFile(join(root, from), join(out, to));
}
await writeFile(join(out, "index.html"), renderPage(releases, { repo: values.repo }));
// Serve the folder as-is (no Jekyll processing).
await writeFile(join(out, ".nojekyll"), "");

console.log(
  `Wrote ${join(out, "index.html")}: ${releases.length} published release(s)` +
    (releases[0] ? `, latest ${releases[0].version}` : ""),
);
