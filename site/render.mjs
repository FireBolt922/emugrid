// Renders the download page from the GitHub releases API response. Pure
// functions only: build.mjs does the reading and writing. Every value that
// comes from a release passes through esc() before it reaches the page.

export const SYSTEMS = [
  {
    key: "macos",
    label: "macOS",
    detail: "macOS 11 or later · Apple silicon and Intel",
    kind: ".dmg",
  },
  {
    key: "windows",
    label: "Windows",
    detail: "Windows 10 or 11 · 64-bit",
    kind: ".zip",
  },
  {
    key: "linux",
    label: "Linux",
    detail: "x86_64 · glibc 2.35 or later (Ubuntu 22.04+, Fedora 36+)",
    kind: ".tar.gz",
  },
];

/** Which download a release file is, from its name; null for anything else. */
export function classifyAsset(name) {
  const n = name.toLowerCase();
  if (n === "sha256sums.txt") return "checksums";
  if (n.includes("macos") && (n.endsWith(".dmg") || n.endsWith(".zip"))) return "macos";
  if (n.includes("windows") && (n.endsWith(".zip") || n.endsWith(".exe"))) return "windows";
  if (n.includes("linux") && n.endsWith(".tar.gz")) return "linux";
  return null;
}

export function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Published releases, newest first, with their downloads by system. */
export function prepareReleases(apiReleases) {
  return apiReleases
    .filter((r) => !r.draft && r.published_at)
    .sort((a, b) => b.published_at.localeCompare(a.published_at))
    .map((r) => {
      const downloads = {};
      for (const a of r.assets ?? []) {
        const key = classifyAsset(a.name);
        // A .dmg beats an older-style .zip for macOS.
        if (key && (!downloads[key] || a.name.endsWith(".dmg"))) {
          downloads[key] = { name: a.name, url: a.browser_download_url, size: formatSize(a.size) };
        }
      }
      return {
        version: r.tag_name.replace(/^v/, ""),
        tag: r.tag_name,
        title: r.name || r.tag_name,
        url: r.html_url,
        date: r.published_at.slice(0, 10),
        prerelease: Boolean(r.prerelease),
        downloads,
      };
    });
}

export function esc(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

const CSP = [
  "default-src 'none'",
  "script-src 'self'",
  "style-src 'self'",
  "font-src 'self'",
  "img-src 'self'",
  "base-uri 'none'",
  "form-action 'none'",
].join("; ");

function badge(release) {
  return release.prerelease ? `<span class="badge">Test build</span>` : "";
}

function primaryButton(latest) {
  if (!latest) {
    return `<a id="primary-download" class="button primary" href="#download">See downloads</a>`;
  }
  const attrs = SYSTEMS.filter((s) => latest.downloads[s.key])
    .map(
      (s) =>
        ` data-href-${s.key}="${esc(latest.downloads[s.key].url)}"` +
        ` data-label-${s.key}="Download for ${esc(s.label)}"`,
    )
    .join("");
  return `<a id="primary-download" class="button primary" href="#download"${attrs}>Choose your download</a>`;
}

function downloadRow(system, release) {
  const file = release.downloads[system.key];
  const cell = file
    ? `<a class="file mono" href="${esc(file.url)}">${esc(file.name)}</a>
            <span class="size">${esc(file.size)}</span>`
    : `<span class="missing">Not in this release</span>`;
  return `
        <li class="system" data-os="${system.key}">
          <div class="system-name">
            <span class="heading">${esc(system.label)}</span>
            <span class="yours" hidden>Your system</span>
            <span class="body-sm muted">${esc(system.detail)}</span>
          </div>
          <div class="system-file">${cell}</div>
        </li>`;
}

function latestSection(latest, repo) {
  if (!latest) {
    return `
    <section id="download" class="section" aria-labelledby="download-title">
      <h2 id="download-title" class="title-xl">No release yet</h2>
      <p class="muted">The first build will appear here when it is published. Follow the
        <a href="https://github.com/${esc(repo)}/releases">releases on GitHub</a> in the meantime.</p>
    </section>`;
  }
  const note = latest.prerelease
    ? `<p class="callout">This is a test build. It may have rough edges; please
        <a href="https://github.com/${esc(repo)}/issues">report what you find</a>.</p>`
    : "";
  const sums = latest.downloads.checksums
    ? `<span aria-hidden="true">·</span> Check your download against
        <a href="${esc(latest.downloads.checksums.url)}">SHA256SUMS.txt</a>`
    : "";
  return `
    <section id="download" class="section" aria-labelledby="download-title">
      <div class="section-head">
        <h2 id="download-title" class="title-xl">Version ${esc(latest.version)}</h2>
        ${badge(latest)}
        <span class="body-sm muted">Released ${esc(latest.date)}</span>
      </div>
      ${note}
      <ul class="systems">${SYSTEMS.map((s) => downloadRow(s, latest)).join("")}
      </ul>
      <p class="body-sm muted links">
        <a href="${esc(latest.url)}">Release notes</a>
        ${sums}
      </p>
    </section>`;
}

function historySection(earlier) {
  if (earlier.length === 0) return "";
  const label = { macos: "macOS", windows: "Windows", linux: "Linux", checksums: "Checksums" };
  const items = earlier
    .map((r) => {
      const files = ["macos", "windows", "linux", "checksums"]
        .filter((key) => r.downloads[key])
        .map(
          (key) =>
            `<a href="${esc(r.downloads[key].url)}" title="${esc(r.downloads[key].name)}">${label[key]}</a>`,
        )
        .join(`<span aria-hidden="true">·</span>`);
      return `
        <li class="release">
          <div class="release-head">
            <a class="heading" href="${esc(r.url)}">${esc(r.version)}</a>
            ${badge(r)}
            <span class="body-sm muted">${esc(r.date)}</span>
          </div>
          <div class="body-sm release-files">${files || `<span class="muted">No downloads</span>`}</div>
        </li>`;
    })
    .join("");
  return `
    <section id="history" class="section" aria-labelledby="history-title">
      <h2 id="history-title" class="title-lg">Earlier releases</h2>
      <ul class="releases">${items}
      </ul>
    </section>`;
}

const INSTALL_HELP = `
    <section id="install" class="section" aria-labelledby="install-title">
      <h2 id="install-title" class="title-lg">Opening an unsigned build</h2>
      <p class="muted">emugrid is not signed or notarized yet, so your system warns you the first
        time you open it. That is expected.</p>
      <div class="grid-3">
        <details class="card">
          <summary class="heading">macOS</summary>
          <ol class="body-sm">
            <li>Open the <code>.dmg</code> and drag emugrid to Applications.</li>
            <li>Open it. When macOS says it cannot verify the app, click <strong>Done</strong>.</li>
            <li>In System Settings → Privacy &amp; Security, click <strong>Open Anyway</strong>.</li>
            <li>If macOS says the app "is damaged", run this once in Terminal:
              <code>xattr -dr com.apple.quarantine /Applications/emugrid.app</code></li>
          </ol>
        </details>
        <details class="card">
          <summary class="heading">Windows</summary>
          <ol class="body-sm">
            <li>Unzip the download and run <code>emugrid.exe</code>. Nothing is installed.</li>
            <li>If SmartScreen says "Windows protected your PC", click <strong>More info</strong>
              → <strong>Run anyway</strong>.</li>
          </ol>
        </details>
        <details class="card">
          <summary class="heading">Linux</summary>
          <ol class="body-sm">
            <li>Unpack it: <code>tar xzf emugrid-*-linux-x86_64.tar.gz</code></li>
            <li>Run <code>./emugrid</code>. Move it into a folder on your <code>PATH</code> to
              keep it, for example <code>~/.local/bin</code>.</li>
            <li>Needs OpenGL and X11 or Wayland, which desktop installs already have.</li>
          </ol>
        </details>
      </div>
    </section>`;

const FEATURES = `
    <section id="features" class="section" aria-labelledby="features-title">
      <h2 id="features-title" class="title-lg">What it does</h2>
      <div class="grid-2">
        <article class="card">
          <h3 class="heading">Grid view</h3>
          <p class="body-sm muted">A live tile for every running device. SDK emulators stream
            over gRPC; other devices refresh about twice a second. Off-screen tiles pause.</p>
        </article>
        <article class="card">
          <h3 class="heading">Focused view</h3>
          <p class="body-sm muted">Click to tap, drag to swipe, right-click to long-press, and type
            straight into Android. Fit, 100% and pixel-perfect zoom.</p>
        </article>
        <article class="card">
          <h3 class="heading">Device list</h3>
          <p class="body-sm muted">Start emulators from a snapshot, cold boot, or wipe. On macOS,
            boot and shut down iOS simulators too.</p>
        </article>
        <article class="card">
          <h3 class="heading">Install APKs</h3>
          <p class="body-sm muted">Drop an <code>.apk</code> on the window to install it on every
            running Android device at once, with results per device.</p>
        </article>
      </div>
      <p class="callout">Needs the Android SDK with <code>platform-tools</code> (adb) and
        <code>emulator</code>; emugrid finds it on its own. Nothing is installed on your devices.
        iOS simulators need Xcode on macOS and are view-only.</p>
    </section>`;

/** Decorative grid of phone tiles, like the app's grid view. */
const HERO_ART = `
      <div class="hero-art" aria-hidden="true">
        ${[
          ["live", "Pixel 8"],
          ["live", "Pixel Tablet"],
          ["polled", "Galaxy S24"],
          ["live", "Pixel 6a"],
          ["stopped", "iPhone 16"],
          ["polled", "Nexus 5X"],
        ]
          .map(
            ([tone, name]) => `<div class="tile">
          <div class="phone"><div class="screen screen-${tone}"></div></div>
          <div class="tile-label"><span class="dot dot-${tone}"></span>${name}</div>
        </div>`,
          )
          .join("\n        ")}
      </div>`;

/** The whole page as an HTML string. */
export function renderPage(releases, { repo }) {
  const [latest, ...earlier] = releases;
  const hint = latest
    ? `Version ${esc(latest.version)}${latest.prerelease ? " · test build" : ""} · free and open source`
    : "Free and open source";
  const gh = `https://github.com/${esc(repo)}`;
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta http-equiv="Content-Security-Policy" content="${CSP}" />
    <title>emugrid</title>
    <meta
      name="description"
      content="See and control all your Android emulators and iOS simulators in one window. A tiny native app for macOS, Windows and Linux."
    />
    <meta name="color-scheme" content="dark light" />
    <link rel="icon" href="icon.svg" type="image/svg+xml" />
    <link rel="stylesheet" href="site.css" />
    <script src="site.js" defer></script>
  </head>
  <body>
    <a class="skip" href="#download">Skip to downloads</a>
    <header class="topbar">
      <a class="brand heading" href="#top"><img src="icon.svg" alt="" width="26" height="26" />emugrid</a>
      <nav class="body-sm" aria-label="Page">
        <a href="#download">Download</a>
        <a class="wide-only" href="#features">Features</a>
        ${earlier.length ? `<a class="wide-only" href="#history">Releases</a>` : ""}
        <a href="${gh}">GitHub</a>
      </nav>
    </header>
    <main id="top">
      <section class="hero">
        <div class="hero-copy">
          <p class="eyebrow">macOS · Windows · Linux</p>
          <h1 class="h1">All your emulators, one window.</h1>
          <p class="lead muted">See and control every Android emulator, and view iOS simulators,
            side by side. A single native binary under 8 MB: no Electron, no browser engine,
            nothing to set up.</p>
          <div class="actions">
            ${primaryButton(latest)}
            <span class="body-sm muted">${hint}</span>
          </div>
        </div>
        ${HERO_ART}
      </section>
      ${latestSection(latest, repo)}
      ${latest ? INSTALL_HELP : ""}
      ${FEATURES}
      ${historySection(earlier)}
    </main>
    <footer class="footer body-sm muted">
      <p>
        <a href="${gh}">GitHub</a>
        <span aria-hidden="true">·</span>
        <a href="${gh}/releases">All releases</a>
        <span aria-hidden="true">·</span>
        <a href="${gh}/issues">Report a problem</a>
        <span aria-hidden="true">·</span>
        <a href="${gh}/blob/main/LICENSE">MIT licence</a>
      </p>
      <p>This page sets no cookies and has no analytics.</p>
    </footer>
  </body>
</html>
`;
}
