import { createRequire } from 'node:module';
import assert from 'node:assert/strict';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const browser = await chromium.launch({
  headless: true,
  ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {}),
});
try {
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.goto(process.argv[2] || 'http://127.0.0.1:4178');
  await page.waitForFunction(() => /ALL CHECKS PASSED|FAIL:/.test(document.querySelector('#results').textContent));
  const output = await page.locator('#results').textContent();
  assert.ok(output.includes('ALL CHECKS PASSED'), output);
  assert.deepEqual(errors, []);
  console.log(output);
} finally {
  await browser.close();
}
