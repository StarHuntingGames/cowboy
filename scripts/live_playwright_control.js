const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ headless: false, args: ['--start-maximized'] });
  const context = await browser.newContext({ viewport: null });
  const page = await context.newPage();
  await page.goto('http://localhost:8080', { waitUntil: 'domcontentloaded', timeout: 120000 });
  console.log('PLAYWRIGHT_READY http://localhost:8080');

  const shutdown = async () => {
    try { await browser.close(); } catch (_) {}
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  setInterval(() => {}, 1000);
})();
