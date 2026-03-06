/**
 * Global setup for Playwright E2E tests.
 *
 * Seeds the application with test documents, schemas, and assets
 * via the REST API using the service token.
 */

const BASE_URL = process.env.BASE_URL || 'http://localhost:3000';
const SERVICE_TOKEN = process.env.SERVICE_TOKEN || 'test-token';

async function ingestDocument(
  slug: string,
  title: string,
  content: string,
  accessLevel: string,
  options: {
    parentSlug?: string;
    order?: number;
    isHidden?: boolean;
    tags?: string[];
  } = {},
) {
  const response = await fetch(`${BASE_URL}/api/v1/ingest`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      service_token: SERVICE_TOKEN,
      slug,
      title,
      content,
      access_level: accessLevel,
      service_owner: 'e2e-tests',
      tags: options.tags ?? ['e2e'],
      order: options.order ?? 0,
      is_hidden: options.isHidden ?? false,
      parent_slug: options.parentSlug ?? null,
    }),
  });

  if (!response.ok) {
    throw new Error(`Failed to ingest ${slug}: ${response.status} ${await response.text()}`);
  }
}

async function ingestSchema(
  name: string,
  schemaType: string,
  version: string,
  content: string,
  status: string = 'stable',
) {
  const response = await fetch(`${BASE_URL}/api/v1/schemas`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      service_token: SERVICE_TOKEN,
      name,
      schema_type: schemaType,
      version,
      content,
      status,
    }),
  });

  if (!response.ok) {
    throw new Error(`Failed to ingest schema ${name}: ${response.status} ${await response.text()}`);
  }
}

export default async function globalSetup() {
  console.log('Seeding test data...');

  // Seed documents
  await ingestDocument(
    'getting-started',
    'Getting Started',
    '# Getting Started\n\nWelcome to **Lekton**, your internal developer portal.\n\n## Quick Start\n\nFollow these steps to get up and running.',
    'public',
    { order: 0, tags: ['guide', 'onboarding'] },
  );

  await ingestDocument(
    'architecture-overview',
    'Architecture Overview',
    '# Architecture Overview\n\nLekton uses a **Rust** backend with Leptos SSR.\n\nSee the [getting started](/docs/getting-started) guide.',
    'public',
    { order: 1 },
  );

  // Nested documents for hierarchy testing
  await ingestDocument(
    'api-docs',
    'API Documentation',
    '# API Documentation\n\nAll available REST endpoints.',
    'public',
    { order: 2 },
  );

  await ingestDocument(
    'api-docs/authentication',
    'Authentication API',
    '# Authentication API\n\nEndpoints for OAuth2/OIDC authentication.',
    'public',
    { parentSlug: 'api-docs', order: 0 },
  );

  await ingestDocument(
    'internal-processes',
    'Internal Processes',
    '# Internal Processes\n\nThis document is only visible to authenticated users with internal access.',
    'internal',
    { order: 3 },
  );

  // Seed a schema
  await ingestSchema(
    'user-api',
    'openapi',
    '1.0.0',
    JSON.stringify({
      openapi: '3.0.0',
      info: { title: 'User API', version: '1.0.0' },
      paths: {
        '/users': {
          get: { summary: 'List users', responses: { '200': { description: 'OK' } } },
        },
      },
    }),
  );

  await ingestSchema('user-api', 'openapi', '1.1.0', JSON.stringify({
    openapi: '3.0.0',
    info: { title: 'User API', version: '1.1.0' },
    paths: {},
  }), 'beta');

  // Wait for Meilisearch indexing
  await new Promise(resolve => setTimeout(resolve, 2000));

  console.log('Test data seeded successfully.');
}
