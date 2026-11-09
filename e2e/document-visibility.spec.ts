import { test, expect, type Page } from '@playwright/test';

// Reading, search, retrieval, chat and Learn share one visibility rule. Demo
// sessions used to read only public documents while chat and Learn treated
// them as logged-in users, so AI answers cited pages the reader could not open.
async function signIn(page: Page, username: string) {
  expect((await page.request.post('/api/auth/login', { data: { username, password: username } })).ok()).toBeTruthy();
}

test.describe('document visibility', () => {
  test('anonymous readers cannot open a logged-in document', async ({ page }) => {
    await page.goto('/docs/logged-in-handbook');
    await expect(page.getByRole('heading', { name: 'Logged-in Handbook' })).toHaveCount(0);
  });

  for (const username of ['demo', 'public']) {
    test(`the ${username} demo session opens a logged-in document`, async ({ page }) => {
      await signIn(page, username);
      await page.goto('/docs/logged-in-handbook');
      await expect(page.getByRole('heading', { name: 'Logged-in Handbook' }).first()).toBeVisible();
    });
  }

  test('demo sessions still cannot open documents above their level', async ({ page }) => {
    await signIn(page, 'demo');
    await page.goto('/docs/internal-processes');
    await expect(page.getByText('only visible to authenticated users with internal access')).toHaveCount(0);
  });
});
