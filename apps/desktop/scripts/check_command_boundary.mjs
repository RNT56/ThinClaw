import { readdir, readFile } from 'node:fs/promises';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const sourceRoot = join(desktopRoot, 'frontend', 'src');
const allowedRawConsumer = join(sourceRoot, 'lib', 'command-client.ts');
const violations = [];

async function inspect(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) {
            if (entry.name !== 'tests') await inspect(path);
            continue;
        }
        if (!['.ts', '.tsx'].includes(extname(path)) || path === allowedRawConsumer || entry.name === 'bindings.ts') {
            continue;
        }

        const source = await readFile(path, 'utf8');
        const bindingImports = source.matchAll(
            /import\s+(?:type\s+)?\{([\s\S]*?)\}\s+from\s+['"][^'"]*bindings(?:\.ts)?['"]/g,
        );
        for (const match of bindingImports) {
            const imports = match[1]
                .split(',')
                .map((part) => part.trim().replace(/^type\s+/, '').split(/\s+as\s+/)[0]);
            if (imports.includes('commands')) {
                violations.push(relative(desktopRoot, path));
                break;
            }
        }
    }
}

await inspect(sourceRoot);
if (violations.length > 0) {
    console.error('Raw generated commands may only be imported by lib/command-client.ts:');
    for (const path of violations) console.error(`  - ${path}`);
    process.exitCode = 1;
}
