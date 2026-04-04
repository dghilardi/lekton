import { type Page } from '@playwright/test';

/**
 * Log in using the demo auth form at /login.
 *
 * The form is handled by /js/login.js (defer script): it POSTs credentials and
 * redirects to / on success. In CI (release binary, slower runner) the fetch can
 * take several seconds, so timeouts are generous.
 */
export async function loginAs(page: Page, username: string, password: string) {
  await page.goto('/login');
  // Ensure the deferred login.js has attached its submit listener
  await page.waitForLoadState('load');
  await page.fill('#login-username', username);
  await page.fill('#login-password', password);
  await page.click('button[type="submit"]');
  // Wait for navigation back to home after successful login (CI can be slow)
  await page.waitForURL('/', { timeout: 30_000 });
  // Wait for WASM hydration: user info is loaded via Leptos LocalResource (client-side only).
  // The user dropdown appears only after WASM fetches /api/auth/me and renders.
  await page.waitForSelector('.dropdown.dropdown-end', { timeout: 30_000 });
}

export async function loginAsDemo(page: Page) {
  await loginAs(page, 'demo', 'demo');
}

export async function loginAsAdmin(page: Page) {
  await loginAs(page, 'admin', 'admin');
}

export async function loginAsPublic(page: Page) {
  await loginAs(page, 'public', 'public');
}

/**
 * Log out via the user menu.
 */
export async function logout(page: Page) {
  // Open dropdown menu
  await page.click('.dropdown.dropdown-end');
  // Click logout button
  await page.click('text=Log Out');
  // Wait for logout to complete
  await page.waitForSelector('a[href="/login"]', { timeout: 10_000 });
}
