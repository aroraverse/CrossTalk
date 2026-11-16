// Capture the real Crosstalk dashboard during the showcase run: the Live wire
// and the Approvals page, one screenshot per page every 2 s, named by time.
// Usage: node capture.mjs <outDir> [minutes]
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const npmRoot = process.env.NPM_GLOBAL || join(process.env.APPDATA || "", "npm", "node_modules");
const puppeteer = require(join(npmRoot, "puppeteer-core"));

const out = process.argv[2];
const minutes = Number(process.argv[3] || 40);
mkdirSync(join(out, "live"), { recursive: true });
mkdirSync(join(out, "approvals"), { recursive: true });

const browser = await puppeteer.launch({
  executablePath: "C:/Program Files/Google/Chrome/Application/chrome.exe",
  defaultViewport: { width: 1440, height: 900, deviceScaleFactor: 2 },
  args: ["--force-color-profile=srgb", "--hide-scrollbars"],
});
const live = await browser.newPage();
await live.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
await live.goto("http://127.0.0.1:7777/#/live", { waitUntil: "networkidle0" });
const appr = await browser.newPage();
await appr.emulateMediaFeatures([{ name: "prefers-color-scheme", value: "dark" }]);
await appr.goto("http://127.0.0.1:7777/#/approvals", { waitUntil: "networkidle0" });

const end = Date.now() + minutes * 60_000;
let n = 0;
while (Date.now() < end) {
  const t = Date.now();
  try {
    await live.screenshot({ path: join(out, "live", `${t}.png`) });
    await appr.screenshot({ path: join(out, "approvals", `${t}.png`) });
    n++;
  } catch (e) { console.error("capture error", e.message); }
  await new Promise((r) => setTimeout(r, Math.max(200, 2000 - (Date.now() - t))));
}
await browser.close();
console.log(`captured ${n} frames per page`);
