import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { Script } from 'node:vm';

const root = fileURLToPath(new URL('../', import.meta.url));
export const options = {
  ecma: 2020,
  // Browser/site properties, IPC fields and window hooks are public contracts.
  // Only lexical bindings are renamed; no eval, decoders or runtime wrappers.
  toplevel: true,
  mangle: { toplevel: true },
  // Preserve the original operations, branches and function boundaries for runtime parity.
  compress: false,
  format: { comments: false, inline_script: true },
  sourceMap: false,
};

export async function protect(source) {
  const { minify } = await import('terser');
  const result = await minify(source, options);
  if (!result.code) throw new Error('JavaScript protection produced empty output');
  new Script(result.code); // Fail the build on invalid generated JavaScript.
  for (const marker of ['__MV_TOKEN__', '__MV_QUALITY__']) {
    if (source.includes(marker) && !result.code.includes(marker)) {
      throw new Error(`Required substitution marker lost: ${marker}`);
    }
  }
  return result.code;
}

async function writeChanged(file, content) {
  let existing;
  try { existing = await readFile(file, 'utf8'); }
  catch (e) { if (e.code !== 'ENOENT') throw e; }
  if (existing !== content) await writeFile(file, content);
}

export async function buildAssets({ dev = false, output = path.join(root, '.generated') } = {}) {
  await mkdir(path.join(output, 'ui'), { recursive: true });
  await mkdir(path.join(output, 'inject'), { recursive: true });
  const html = await readFile(path.join(root, 'ui/index.html'), 'utf8');
  const blocks = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
  if (blocks.length !== 1) throw new Error('Expected exactly one inline UI script');
  const js = dev ? blocks[0][1] : await protect(blocks[0][1]);
  const generated = html.replace(blocks[0][0], `<script>${js}</script>`);
  await writeChanged(path.join(output, 'ui/index.html'), generated);
  for (const name of (await readdir(path.join(root, 'src-tauri/src/inject'))).sort()) {
    if (!name.endsWith('.js')) continue;
    const source = await readFile(path.join(root, 'src-tauri/src/inject', name), 'utf8');
    await writeChanged(path.join(output, 'inject', name), dev ? source : await protect(source));
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await buildAssets({ dev: process.argv.includes('--dev') });
}
