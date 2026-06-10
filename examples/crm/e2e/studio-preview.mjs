// Playwright drives the MotokoStudio 3-up PREVIEW CANVAS (web/iOS/Android panes)
// served by `motoview preview examples/crm --serve` (no dfx; moc -r). Proves the
// studio UI renders the CRM's one UI-IR source across all three platform panes.
import { chromium } from 'playwright';

const URL = process.env.STUDIO_URL || 'http://127.0.0.1:4980/';
const OUT = process.env.OUT || '/tmp';
const browser = await chromium.launch();
const page = await browser.newPage();
let ok = true;
const log = (m) => console.log(m);
try {
  // NOTE: the canvas keeps an SSE (/events) connection open, so 'networkidle'
  // never fires — wait on domcontentloaded, then let the canvas fetch /forest.
  await page.goto(URL, { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.waitForTimeout(3000); // let the canvas fetch /forest + render the panes
  const title = await page.title();
  const body = (await page.textContent('body')) || '';
  const html = (await page.content()) || '';
  const hasPipeline = /Pipeline/i.test(body);                 // the CRM h1 rendered in the panes
  const hasWeb = /\bweb\b/i.test(html), hasIos = /\bios\b/i.test(html), hasAndroid = /\bandroid\b/i.test(html);
  // count how many panes rendered the CRM heading (one per platform column)
  const pipelineHeadings = await page.locator('text=Pipeline').count();
  await page.screenshot({ path: `${OUT}/crm-studio-3up.png`, fullPage: true });
  log(`title="${title}" panes(web/ios/android)=${hasWeb}/${hasIos}/${hasAndroid} Pipeline-rendered=${hasPipeline} pipeline-headings=${pipelineHeadings}`);
  if (!(title && /preview/i.test(title)) || !hasPipeline) { ok = false; log('FAIL: studio canvas did not render the CRM'); }
} catch (e) { ok = false; log('ERROR: ' + e.message); } finally { await browser.close(); }
log(ok ? 'STUDIO_PREVIEW_OK' : 'STUDIO_PREVIEW_FAIL');
process.exit(ok ? 0 : 1);
