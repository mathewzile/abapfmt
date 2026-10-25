// Copies the release build of abapfmt for this platform into bin/<platform>-<arch>/ (run
// `cargo build --release -p abapfmt_cli` first). The CI workflow fills bin/ for all platforms.
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
const dest = path.resolve(here, '../bin', `${process.platform}-${process.arch}`);
fs.mkdirSync(dest, { recursive: true });
fs.copyFileSync(source, path.join(dest, exe));
console.log(`bundled ${path.join(`${process.platform}-${process.arch}`, exe)}`);
