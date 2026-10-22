import { createHash } from 'node:crypto';
import { chmod, mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const destination = resolve(root, 'src-tauri/resources/displayplacer/displayplacer');
const version = 'v1.4.0';
const assets = {
  arm64: {
    name: 'displayplacer-apple-v140',
    sha256: '0572c3d2918e47c7e0b9d7723907864e2ea2b53b9d3b02379769fffcf44f7ea0',
  },
  x64: {
    name: 'displayplacer-intel-v140',
    sha256: '13ec0351ed7849b22e945974f1d4ac91eca30b38b09ec962c497feb8297eac2b',
  },
};

if (process.platform !== 'darwin') {
  console.log('displayplacer is only bundled in macOS builds.');
  process.exit(0);
}

const asset = assets[process.arch];
if (!asset) throw new Error(`Unsupported macOS architecture for displayplacer: ${process.arch}`);

await mkdir(dirname(destination), { recursive: true });
const temp = `${destination}.download`;
try {
  const cached = await readFile(destination);
  const cachedHash = createHash('sha256').update(cached).digest('hex');
  if (cachedHash === asset.sha256) {
    console.log(`Using SHA-256 verified displayplacer ${version} (${process.arch}) already present.`);
    process.exit(0);
  }
} catch {
  // No valid cached helper; download the pinned upstream asset below.
}

try {
  const response = await fetch(
    `https://github.com/jakehilborn/displayplacer/releases/download/${version}/${asset.name}`,
    { redirect: 'follow' },
  );
  if (!response.ok) throw new Error(`Download failed: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const actualHash = createHash('sha256').update(bytes).digest('hex');
  if (actualHash !== asset.sha256) throw new Error(`SHA-256 mismatch for ${asset.name}`);
  await writeFile(temp, bytes, { mode: 0o755 });
  await chmod(temp, 0o755);
  await rename(temp, destination);
  console.log(`Bundled displayplacer ${version} (${process.arch}, SHA-256 verified).`);
} catch (error) {
  await rm(temp, { force: true });
  throw error;
}
