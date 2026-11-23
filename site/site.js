// Points the main button at the visitor's system and marks that row.
// The page works without this script: the button then jumps to the list.
(() => {
  const ua = navigator.userAgent;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  const text = `${platform} ${ua}`;
  let os = null;
  if (/iPhone|iPad|Android/i.test(text)) os = null; // phones get the full list
  else if (/Mac/i.test(text)) os = "macos";
  else if (/Win/i.test(text)) os = "windows";
  else if (/Linux|X11/i.test(text)) os = "linux";
  if (!os) return;

  const button = document.getElementById("primary-download");
  const href = button && button.getAttribute(`data-href-${os}`);
  if (href) {
    button.href = href;
    button.textContent = button.getAttribute(`data-label-${os}`);
  }
  const row = document.querySelector(`.system[data-os="${os}"]`);
  if (row) {
    row.classList.add("is-yours");
    const tag = row.querySelector(".yours");
    if (tag) tag.hidden = false;
  }
})();
