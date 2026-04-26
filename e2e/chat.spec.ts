import { test, expect } from '@playwright/test';
import { loginAsDemo } from './helpers/auth';

test.describe('Chat page', () => {
  test('page loads without errors', async ({ page }) => {
    // loginAsDemo + goto + networkidle can take >30s on slow CI runners
    test.setTimeout(90_000);

    await loginAsDemo(page);
    await page.goto('/chat');
    await page.waitForLoadState('networkidle');

    // The page should load and show either the chat UI or a "not configured" notice.
    // The app shell is confirmed by the user dropdown rendered after WASM hydration
    // (DaisyUI navbar uses <div class="navbar">, not a semantic <nav> element).
    await expect(page.locator('.dropdown.dropdown-end')).toBeVisible({ timeout: 30_000 });
  });

  test('shows input area or unavailable notice', async ({ page }) => {
    test.setTimeout(90_000);

    await loginAsDemo(page);
    await page.goto('/chat');
    await page.waitForLoadState('networkidle');

    // The chat page renders one of two states:
    // - textarea/input when LLM is configured (full chat UI)
    // - a notice text when not configured
    // Both are valid; we assert the page is not blank.
    const hasInput = await page.locator('textarea').count() > 0;
    const hasNotice = await page.locator('text=/not available|not configured|unavailable/i').count() > 0;

    expect(hasInput || hasNotice, 'chat page should show either input or unavailability notice').toBeTruthy();
  });
});
