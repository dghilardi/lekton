import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const { values } = parseArgs({ options: { port: { type: 'string', default: '4173' } } });
const root = resolve(fileURLToPath(new URL('./dist/', import.meta.url)));
const types = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.js': 'text/javascript', '.svg': 'image/svg+xml' };
const base = '/lekton/';

// A local-only preview under the same project prefix used by GitHub Pages.
await import('./build.mjs');
createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    if (pathname === '/') {
      response.writeHead(302, { Location: base });
      response.end();
      return;
    }
    if (!pathname.startsWith(base)) throw new Error('Not found');
    let filename = resolve(root, pathname.slice(base.length));
    if (filename !== root && !filename.startsWith(root + sep)) throw new Error('Not found');
    if ((await stat(filename)).isDirectory()) {
      if (!pathname.endsWith('/')) {
        response.writeHead(302, { Location: pathname + '/' });
        response.end();
        return;
      }
      filename = resolve(filename, 'index.html');
    }
    const body = await readFile(filename);
    response.writeHead(200, { 'Content-Type': types[extname(filename)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
    response.end(body);
  } catch {
    response.writeHead(404, { 'Content-Type': 'text/plain' });
    response.end('Not found');
  }
}).listen(Number(values.port), '127.0.0.1', () => console.log(`Website preview: http://localhost:${values.port}${base}`));
