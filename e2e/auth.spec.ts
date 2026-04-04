import { test, expect } from '@playwright/test';
import { loginAsAdmin, loginAsDemo, logout } from './helpers/auth';

test.describe('Authentication', () => {
  test('login page shows form', async ({ page }) => {
    await page.goto('/login');
    await expect(page.locator('#login-username')).toBeVisible();
    await expect(page.locator('#login-password')).toBeVisible();
    await expect(page.locator('button[type="submit"]', { hasText: 'Sign In' })).toBeVisible();
  });

  test('demo login succeeds', async ({ page }) => {
    await loginAsDemo(page);
    // User menu should show the user's name (rendered by WASM LocalResource)
    await expect(page.locator('text=Demo User')).toBeVisible({ timeout: 30_000 });
  });

  test('admin login shows admin badge', async ({ page }) => {
    await loginAsAdmin(page);
    // Should show "Admin" badge in user menu area (rendered by WASM LocalResource)
    await expect(page.locator('text=Admin').first()).toBeVisible({ timeout: 30_000 });
  });

  test('logout clears session', async ({ page }) => {
    await loginAsDemo(page);
    // Wait for WASM to render the user name before proceeding
    await expect(page.locator('text=Demo User')).toBeVisible({ timeout: 30_000 });
    await logout(page);
    // "Log In" link should reappear
    await expect(page.locator('a[href="/login"]')).toBeVisible();
  });

  test('invalid credentials shows error', async ({ page }) => {
    await page.goto('/login');
    await page.waitForLoadState('load'); // ensure login.js defer script ran
    await page.fill('#login-username', 'demo');
    await page.fill('#login-password', 'wrongpassword');
    await page.click('button[type="submit"]');
    // Error message: login.js removes 'hidden' class on failed fetch
    await expect(page.locator('#login-error')).toBeVisible({ timeout: 10_000 });
  });
});
