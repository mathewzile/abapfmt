// Copies the release build of abapfmt for this platform into bin/ (run `cargo build --release -p abapfmt_cli` first).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const exe = process.platform === 'win32' ? 'abapfmt.exe' : 'abapfmt';
const source = path.resolve(here, '../../../target/release', exe);
if (!fs.existsSync(source)) {
  console.error(`missing ${source}; run: cargo build --release -p abapfmt_cli`);
  process.exit(1);
}
const dest = path.resolve(here, '../bin');
fs.mkdirSync(dest, { recursive: true });
fs.copyFileSync(source, path.join(dest, exe));
console.log(`bundled ${exe}`);
