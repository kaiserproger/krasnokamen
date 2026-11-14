const { firefox } = require('playwright-core');

const path = require('path');
const fs = require('fs');
const FIREFOX = process.env.KRASNOKAMEN_WINGUI_FIREFOX || path.join(process.env.HOME, '.cache/ms-playwright/firefox-1555/firefox/firefox');
const URL = process.env.KRASNOKAMEN_WINGUI_URL || 'http://localhost:8006/';
const SHOTS = process.env.KRASNOKAMEN_WINGUI_SHOTS || '.analysis/winqa-shots';
const readline = require('readline');

let browser = null, page = null;
async function ensure() {
  if (page) return;
  browser = await firefox.launch({ executablePath: FIREFOX, headless: true, args: ['-no-remote'] });
  page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.goto(URL, { waitUntil: 'domcontentloaded', timeout: 20000 });
  await page.waitForSelector('#noVNC_canvas, canvas', { timeout: 30000 });
}

async function handle(c) {
  switch (c.cmd) {
    case 'waitconn':
      await ensure();
      await page.waitForFunction(
        () => document.documentElement.classList.contains('noVNC_connected'),
        null, { timeout: c.timeout || 300000 });
      return { ok: true };
    case 'shot': {
      await ensure();
      fs.mkdirSync(SHOTS, { recursive: true });
      const p = path.join(SHOTS, c.tag + '.png');
      await page.screenshot({ path: p });
      return { ok: true, path: p };
    }
    case 'click':
      await ensure();
      await page.mouse.click(c.x, c.y);
      return { ok: true };
    case 'key':
      await ensure();
      await page.keyboard.press(c.key);
      return { ok: true };
    case 'type':
      await ensure();
      await page.keyboard.type(c.text, { delay: c.delay || 60 });
      return { ok: true };
    case 'quit':
      if (browser) await browser.close();
      browser = page = null;
      return { ok: true };
    default:
      throw new Error('unknown cmd ' + c.cmd);
  }
}

const rl = readline.createInterface({ input: process.stdin });
console.log('READY');
rl.on('line', async (line) => {
  if (!line.trim()) return;
  let c = {};
  try {
    c = JSON.parse(line);
    const r = await handle(c);
    console.log(JSON.stringify({ id: c.id || null, ...r }));
  } catch (e) {
    console.log(JSON.stringify({ id: c.id || null, ok: false, error: String((e && e.message) || e) }));
  }
});
