import { cp, mkdir, readFile, readdir, rm, writeFile, access } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';

const root = resolve('..');
const content = resolve('src/content');
const publicDir = resolve('public');
await mkdir(content, { recursive: true });
await mkdir(publicDir, { recursive: true });

// Vercel builds from the `site/` directory, so the monorepo files one level
// above are not available there. The generated content and docs are checked
// in for that deployment path; keep them as-is when the source tree is absent.
try {
  await access(join(root, 'README.md'));
} catch {
  console.warn('Monorepo sources are unavailable; using checked-in site content.');
  process.exit(0);
}

await cp(join(root, 'README.md'), join(content, 'README.md'));
await cp(join(root, 'docs/src/django-comparison.md'), join(content, 'django-comparison.md'));
await cp(join(root, 'docs/src/benchmarks.md'), join(content, 'benchmarks.md'));
await cp(join(root, 'CHANGELOG.md'), join(content, 'CHANGELOG.md'));

const docs = [];
async function walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) await walk(path);
    else if (entry.name.endsWith('.md')) docs.push(path);
  }
}
await walk(join(root, 'docs/src'));
const full = [await readFile(join(root, 'README.md'), 'utf8')];
for (const path of docs.sort()) full.push(`\n\n<!-- ${path.slice(root.length + 1)} -->\n\n${await readFile(path, 'utf8')}`);
full.push(`\n\n<!-- PLAN.md -->\n\n${await readFile(join(root, 'PLAN.md'), 'utf8')}`);
await writeFile(join(publicDir, 'llms-full.txt'), full.join(''));
await writeFile(join(publicDir, 'llms.txt'), '# Djangors\n\nThe Django of Rust: batteries-included web development with Rust.\n\n- [Home](/)\n- [Engineering Blog](/blog)\n- [Django comparison](/compare)\n- [Benchmarks](/benchmarks)\n- [Full documentation](/docs/)\n- [Full Markdown corpus](/llms-full.txt)\n');

execFileSync('mdbook', ['build', join(root, 'docs')], { stdio: 'inherit' });
execFileSync(join(root, 'docs/scripts/copy-raw-markdown.sh'), { stdio: 'inherit' });
await rm(join(publicDir, 'docs'), { recursive: true, force: true });
await cp(join(root, 'docs/book'), join(publicDir, 'docs'), { recursive: true });
