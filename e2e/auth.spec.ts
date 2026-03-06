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
    // User menu should show the user's name
    await expect(page.locator('text=Demo User')).toBeVisible();
  });

  test('admin login shows admin badge', async ({ page }) => {
    await loginAsAdmin(page);
    // Should show "Admin" badge in user menu area
    await expect(page.locator('text=Admin').first()).toBeVisible();
  });

  test('logout clears session', async ({ page }) => {
    await loginAsDemo(page);
    await expect(page.locator('text=Demo User')).toBeVisible();
    await logout(page);
    // "Log In" link should reappear
    await expect(page.locator('a[href="/login"]')).toBeVisible();
  });

  test('invalid credentials shows error', async ({ page }) => {
    await page.goto('/login');
    await page.fill('#login-username', 'demo');
    await page.fill('#login-password', 'wrongpassword');
    await page.click('button[type="submit"]');
    // Error message should appear
    const errorAlert = page.locator('#login-error, .alert-error');
    await expect(errorAlert).toBeVisible({ timeout: 5_000 });
  });
});
