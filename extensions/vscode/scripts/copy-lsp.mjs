import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = fileURLToPath(new URL('.', import.meta.url));
const projectRoot = resolve(__dirname, '../../../');
const destDir = resolve(__dirname, '../server');

mkdirSync(destDir, { recursive: true });

const binaries = ['xenomorph_lsp', 'xenomorph_lsp.exe'];
let copied = false;

for (const bin of binaries) {
    const src = join(projectRoot, 'target', 'release', bin);
    if (existsSync(src)) {
        copyFileSync(src, join(destDir, bin));
        console.log(`✓ Copied ${bin} → server/`);
        copied = true;
    }
}

if (!copied) {
    console.error('✗ No LSP binary found in target/release/');
    process.exit(1);
}
