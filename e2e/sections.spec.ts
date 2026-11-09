import { test, expect, type APIRequestContext } from '@playwright/test';

const SERVICE_TOKEN = process.env.LKN__AUTH__SERVICE_TOKEN || 'test-token';

async function ingest(
  request: APIRequestContext,
  slug: string,
  title: string,
  parentSlug: string | null = null,
  // Hidden by default, so other specs' menus stay as seeded.
  { accessLevel = 'public', isHidden = true } = {},
) {
  const response = await request.post('/api/v1/ingest', {
    data: {
      service_token: SERVICE_TOKEN,
      slug,
      source_path: `sections/${slug.replace(/\/$/, '/index')}.md`,
      source_id: 'e2e-sections',
      title,
      content: `# ${title}\n\nSection body.`,
      access_level: accessLevel,
      service_owner: 'e2e-tests',
      tags: ['e2e'],
      order: 0,
      is_hidden: isHidden,
      parent_slug: parentSlug,
    },
  });
  expect(response.ok(), await response.text()).toBeTruthy();
}

test.describe('Section landing pages', () => {
  test('a section index sent with a trailing slash is served at the section URL', async ({ page, request }) => {
    // What earlier lekton-sync releases sent for a prefixed root index.md.
    await ingest(request, 'e2e-section/', 'Section Index', 'e2e-section');
    await ingest(request, 'e2e-section/child', 'Section Child', 'e2e-section');

    const response = await page.goto('/docs/e2e-section');
    expect(response?.status()).toBe(200);
    await expect(page.getByRole('heading', { level: 1, name: 'Section Index' })).toBeVisible();
    await expect(page.getByText('Section body.')).toBeVisible();
  });

  test('a section without an index lists its pages under a heading, with no edit date', async ({ page, request }) => {
    // Listings skip hidden pages; an internal one stays out of anonymous menus.
    await ingest(request, 'api-docs/e2e-listing/first/page', 'Listed Page', null, { accessLevel: 'internal', isHidden: false });
    expect((await page.request.post('/api/auth/login', { data: { username: 'admin', password: 'admin' } })).ok()).toBeTruthy();

    await page.goto('/docs/api-docs/e2e-listing');
    await expect(page.getByRole('heading', { level: 1, name: 'E2e Listing' })).toBeVisible();
    await expect(page.getByText('1 page in this section.')).toBeVisible();
    await expect(page.locator('main').getByRole('link', { name: /First/ })).toHaveAttribute('href', '/docs/api-docs/e2e-listing/first');
    await expect(page.getByText('Last updated')).toHaveCount(0);
  });
});

test('Get Started opens the onboarding page', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('link', { name: 'Get Started' })).toHaveAttribute('href', '/docs/getting-started');
});
