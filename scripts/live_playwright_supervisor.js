const { chromium } = require("playwright");

const TARGET_URL = process.env.LIVE_TEST_URL || "http://localhost:8080";
const DISPLAY = process.env.DISPLAY || ":20.0";
const RELAUNCH_DELAY_MS = 1200;

let browser = null;
let stopping = false;

async function launchBrowser() {
  if (stopping) return;

  try {
    browser = await chromium.launch({
      headless: false,
      args: ["--start-maximized"],
    });

    browser.on("disconnected", () => {
      if (stopping) return;
      console.error("PLAYWRIGHT_BROWSER_DISCONNECTED relaunching...");
      setTimeout(() => {
        launchBrowser().catch((error) => {
          console.error("PLAYWRIGHT_RELAUNCH_ERROR", error && error.stack ? error.stack : error);
        });
      }, RELAUNCH_DELAY_MS);
    });

    const context = await browser.newContext({ viewport: null });
    const page = await context.newPage();
    await page.goto(TARGET_URL, { waitUntil: "domcontentloaded", timeout: 120000 });
    console.log(`PLAYWRIGHT_READY ${TARGET_URL} display=${DISPLAY}`);
  } catch (error) {
    console.error("PLAYWRIGHT_LAUNCH_ERROR", error && error.stack ? error.stack : error);
    if (!stopping) {
      setTimeout(() => {
        launchBrowser().catch((nextError) => {
          console.error("PLAYWRIGHT_RETRY_ERROR", nextError && nextError.stack ? nextError.stack : nextError);
        });
      }, RELAUNCH_DELAY_MS);
    }
  }
}

async function shutdown() {
  stopping = true;
  if (browser) {
    try {
      await browser.close();
    } catch (_error) {}
  }
  process.exit(0);
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);

setInterval(() => {}, 1000);

launchBrowser().catch((error) => {
  console.error("PLAYWRIGHT_FATAL", error && error.stack ? error.stack : error);
});
