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

    // With features.rag enabled, the chat page renders one of two states:
    // - textarea/input when LLM is configured (full chat UI)
    // - a notice text when not configured
    // With features.rag disabled, /chat is intentionally routed to NotFound.
    const hasInput = await page.locator('textarea').count() > 0;
    const hasNotice = await page.locator('text=/not available|not configured|unavailable/i').count() > 0;
    const hasNotFound = await page.getByRole('heading', { level: 1, name: 'Page not found' }).count() > 0;

    expect(
      hasInput || hasNotice || hasNotFound,
      'chat page should show chat UI, an unavailability notice, or the feature-disabled 404 page',
    ).toBeTruthy();
  });

  test('lmltfy link types and sends the question, then is consumed', async ({ page }) => {
    test.setTimeout(90_000);

    await loginAsDemo(page);
    await page.goto('/chat?foo=bar&lmltfy=How%20do%20I%20configure%20OIDC%3F');
    await page.waitForLoadState('networkidle');
    test.skip(await page.locator('textarea').count() === 0, 'chat UI not available');

    // Reloading must not replay it, so the parameter goes as soon as it starts.
    await expect(page).toHaveURL(/\/chat\?foo=bar$/);
    // Typing the question and sending it takes a few seconds.
    await expect(page.locator('.whitespace-pre-wrap', { hasText: 'How do I configure OIDC?' }))
      .toBeVisible({ timeout: 20_000 });
    await expect(page.locator('textarea')).toHaveValue('');
  });

  test('lmltfy64 link decodes, types and sends the question, then is consumed', async ({ page }) => {
    test.setTimeout(90_000);

    await loginAsDemo(page);
    // URL-safe base64 of "How do I configure OIDC?".
    await page.goto('/chat?foo=bar&lmltfy64=SG93IGRvIEkgY29uZmlndXJlIE9JREM_');
    await page.waitForLoadState('networkidle');
    test.skip(await page.locator('textarea').count() === 0, 'chat UI not available');

    await expect(page).toHaveURL(/\/chat\?foo=bar$/);
    await expect(page.locator('.whitespace-pre-wrap', { hasText: 'How do I configure OIDC?' }))
      .toBeVisible({ timeout: 20_000 });
    await expect(page.locator('textarea')).toHaveValue('');
  });
});

test('an answer lists its sources openly and the session shows when it was used', async ({ page }) => {
  test.setTimeout(90_000);
  await loginAsDemo(page);
  await page.goto('/chat');
  await page.waitForLoadState('networkidle');
  test.skip(await page.locator('textarea').count() === 0, 'chat UI not available');

  const input = page.locator('[data-lekton-chat-input]');
  await input.fill('How do I get started?');
  await input.press('Enter');
  const sources = page.getByRole('region', { name: 'Sources' }).last();
  await expect(sources).toBeVisible({ timeout: 30_000 });
  // Visible links, not a collapsed disclosure.
  await expect(sources.getByRole('link').first()).toBeVisible();
  await expect(page.locator('[data-lekton-chat-session]').first()).toContainText(/\d{1,2}:\d{2}/);
});

test('stopping is limited to the owner and reports whether an answer was running', async ({ page, request }) => {
  await loginAsDemo(page);
  const sessions = await page.request.get('/api/v1/rag/sessions');
  test.skip(!sessions.ok(), 'chat not enabled');
  // Someone else's (or no) session reads as missing.
  expect((await request.post('/api/v1/rag/sessions/not-a-session/stop')).status()).toBe(401);
  expect((await page.request.post('/api/v1/rag/sessions/not-a-session/stop')).status()).toBe(404);
  const own = (await sessions.json())[0];
  test.skip(!own, 'no chat session yet');
  expect(await (await page.request.post(`/api/v1/rag/sessions/${own.id}/stop`)).json()).toEqual({ stopped: false });
});

// Needs a model that streams slowly: the e2e stub answers "#long" that way.
test('the stop button ends a long answer and keeps what was written', async ({ page }) => {
  test.skip(!process.env.LEKTON_SLOW_LLM, 'needs the slow streaming stub (LEKTON_SLOW_LLM=1)');
  test.setTimeout(90_000);
  await loginAsDemo(page);
  await page.goto('/chat');
  const input = page.locator('[data-lekton-chat-input]');
  await expect(input).toBeEnabled({ timeout: 30_000 });
  await input.fill('Explain incidents #long');
  await input.press('Enter');
  await page.getByRole('button', { name: 'Stop the answer' }).click({ timeout: 30_000 });
  await expect(page.getByText('You stopped this answer.')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole('button', { name: 'Regenerate' })).toBeVisible();
  // The server saved the partial answer as stopped.
  const [latest] = await (await page.request.get('/api/v1/rag/sessions')).json();
  await page.goto(`/chat?session=${latest.id}`);
  await expect(page.getByText('You stopped this answer.')).toBeVisible({ timeout: 30_000 });
});
