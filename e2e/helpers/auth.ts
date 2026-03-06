import { type Page } from '@playwright/test';

/**
 * Log in using the demo auth form at /login.
 */
export async function loginAs(page: Page, username: string, password: string) {
  await page.goto('/login');
  await page.fill('#login-username', username);
  await page.fill('#login-password', password);
  await page.click('button[type="submit"]');
  // Wait for navigation back to home after successful login
  await page.waitForURL('/', { timeout: 10_000 });
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
  await page.waitForSelector('a[href="/login"]', { timeout: 5_000 });
}
