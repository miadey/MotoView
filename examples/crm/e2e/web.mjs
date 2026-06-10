// Playwright web E2E for the MotoView CRM — drives the LIVE deployed canister
// in a real browser, exercising the full server-driven loop:
//   server render -> @click event (toggleCreate) -> http_request_update -> DOM patch
//   -> secure form submit (createDeal) -> on-chain state mutation -> re-render.
// Proves the CRM RUNS on web. Usage: CRM_URL=... node web.mjs
import { chromium } from 'playwright';

const URL = process.env.CRM_URL || 'http://ucwa4-rx777-77774-qaada-cai.localhost:4955/';
const OUT = process.env.OUT || '/tmp';

const browser = await chromium.launch();
const page = await browser.newPage();
let ok = true;
const log = (m) => console.log(m);

try {
  await page.goto(URL, { waitUntil: 'networkidle', timeout: 30000 });
  await page.waitForSelector('h1', { timeout: 15000 });
  const h1 = (await page.textContent('h1'))?.trim();
  const columns = await page.locator('.kanban-col').count();
  const before = await page.locator('.deal-card').count();
  log(`rendered: h1="${h1}" kanban-columns=${columns} seeded-deals=${before}`);
  await page.screenshot({ path: `${OUT}/crm-web-board.png` });
  if (!h1 || !h1.toLowerCase().includes('pipeline') || columns < 1) { ok = false; log('FAIL: board did not render'); }

  // 1) @click event: open the create form (server round-trip)
  await page.click('button:has-text("New deal")');
  await page.waitForSelector('input[name="title"]', { timeout: 12000 });
  log('create form opened (toggleCreate event applied)');

  // 2) fill + submit the secure form
  await page.fill('input[name="title"]', 'Acme renewal');
  await page.fill('input[name="company"]', 'Acme Corp');
  await page.fill('input[name="contact"]', 'Jane Doe');
  await page.fill('input[name="value"]', '42000');
  await page.click('button:has-text("Create deal")');

  // 3) assert the on-chain state mutated -> a new card appeared
  await page.waitForFunction(
    (b) => document.querySelectorAll('.deal-card').length > b, before, { timeout: 15000 });
  const after = await page.locator('.deal-card').count();
  const hasNew = await page.locator('text=Acme renewal').count();
  await page.screenshot({ path: `${OUT}/crm-web-after.png` });
  log(`after createDeal: deals ${before} -> ${after}; "Acme renewal" present=${hasNew}`);
  if (!(after > before) || hasNew < 1) { ok = false; log('FAIL: new deal did not appear'); }
} catch (e) {
  ok = false; log('ERROR: ' + e.message);
} finally {
  await browser.close();
}
log(ok ? 'WEB_E2E_OK' : 'WEB_E2E_FAIL');
process.exit(ok ? 0 : 1);
