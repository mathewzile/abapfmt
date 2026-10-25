// Fails unless bin/ holds a build for every platform the extension supports.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const platforms = {
  'win32-x64': 'abapfmt.exe',
  'linux-x64': 'abapfmt',
  'darwin-x64': 'abapfmt',
  'darwin-arm64': 'abapfmt',
};
const missing = Object.entries(platforms)
  .map(([dir, exe]) => path.join('bin', dir, exe))
  .filter((p) => !fs.existsSync(path.resolve(here, '..', p)));
if (missing.length) {
  console.error(`missing builds: ${missing.join(', ')}`);
  process.exit(1);
}
console.log(`found builds for ${Object.keys(platforms).join(', ')}`);
