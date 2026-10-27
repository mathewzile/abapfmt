// Fails unless the extension and the Rust workspace have the same version, and, when run for a tag
// (GITHUB_REF_NAME = v<version>), unless the tag names that version.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const extension = JSON.parse(fs.readFileSync(path.resolve(here, '../package.json'), 'utf8')).version;
const cargo = fs.readFileSync(path.resolve(here, '../../../Cargo.toml'), 'utf8');
const workspace = /\[workspace\.package\][^[]*?^version\s*=\s*"([^"]+)"/m.exec(cargo)?.[1];

if (!workspace) {
  console.error('no version in [workspace.package] of Cargo.toml');
  process.exit(1);
}
if (extension !== workspace) {
  console.error(`package.json has version ${extension}, Cargo.toml has ${workspace}`);
  process.exit(1);
}
const ref = process.env.GITHUB_REF_NAME ?? '';
if (ref.startsWith('v') && ref !== `v${workspace}`) {
  console.error(`tag ${ref} does not match version ${workspace}`);
  process.exit(1);
}
console.log(`version ${workspace}`);
