import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const ignoredDirectories = new Set(['node_modules', 'target', 'coverage', 'out', 'dist']);
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*)?(?:\+[0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*)?$/;

function findFiles(directory) {
    const files = [];
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) {
            // Exclude generated files, hidden directories, and nested repositories/submodules.
            if (!entry.name.startsWith('.') && !ignoredDirectories.has(entry.name) && !existsSync(join(path, '.git'))) {
                files.push(...findFiles(path));
            }
        } else if (entry.isFile() && ['package.json', 'Cargo.toml', 'Cargo.lock'].includes(entry.name)) {
            files.push(path);
        }
    }
    return files.sort();
}

function tomlField(section, name) {
    return section.match(new RegExp(`^${name}\\s*=\\s*"([^"]+)"`, 'm'))?.[1];
}

try {
    const args = process.argv.slice(2);
    const check = args[0] === '--check';
    const [version, ...extra] = check ? args.slice(1) : args;
    if (!version || !semver.test(version) || extra.length) {
        throw new Error('Usage: node scripts/update-versions.mjs [--check] <semver> (for example, 1.2.3)');
    }

    const files = findFiles(root);
    const updates = new Map();
    const crates = new Map();
    const mismatches = [];
    let packages = 0;

    function recordVersion(path, current) {
        packages++;
        if (current !== version) {
            mismatches.push(`${relative(root, path)}: ${current ?? '(missing version)'} (expected ${version})`);
        }
    }

    for (const path of files) {
        const original = readFileSync(path, 'utf8');
        if (path.endsWith('package.json')) {
            const manifest = JSON.parse(original);
            recordVersion(path, manifest.version);
            manifest.version = version;
            const indent = original.match(/\n([\t ]+)"/)?.[1] ?? '    ';
            const newline = original.includes('\r\n') ? '\r\n' : '\n';
            updates.set(path, `${JSON.stringify(manifest, null, indent)}\n`.replaceAll('\n', newline));
        } else if (path.endsWith('Cargo.toml')) {
            const section = original.match(/^\[package\][\s\S]*?(?=^\[|(?![\s\S]))/m)?.[0];
            if (!section) continue; // The root Cargo manifest defines a virtual workspace.
            const name = tomlField(section, 'name');
            const current = tomlField(section, 'version');
            if (!name || !current) throw new Error(`Missing package name/version in ${relative(root, path)}`);
            recordVersion(path, current);
            crates.set(name, version);
            updates.set(path, original.replace(section, section.replace(/^(version\s*=\s*")[^"]+(".*)$/m, `$1${version}$2`)));
        }
    }

    for (const path of files.filter((path) => path.endsWith('Cargo.lock'))) {
        const original = readFileSync(path, 'utf8');
        const updated = original.replace(/^\[\[package\]\][\s\S]*?(?=^\[\[package\]\]|(?![\s\S]))/gm, (section) => {
            const name = tomlField(section, 'name');
            if (!crates.has(name) || tomlField(section, 'source')) return section;
            const current = tomlField(section, 'version');
            if (!current) throw new Error(`Missing lockfile version for ${name} in ${relative(root, path)}`);
            if (current !== version) mismatches.push(`${relative(root, path)} (${name}): ${current} (expected ${version})`);
            return section.replace(/^(version\s*=\s*")[^"]+(".*)$/m, `$1${version}$2`);
        });
        updates.set(path, updated);
    }

    if (check) {
        if (mismatches.length) throw new Error(`Package versions do not match:\n${mismatches.join('\n')}`);
        console.log(`All ${packages} package versions and Cargo lockfile entries match ${version}.`);
    } else {
        // Prepare every change before writing, so invalid input/manifests cannot cause a partial update.
        for (const [path, updated] of updates) {
            if (updated !== readFileSync(path, 'utf8')) writeFileSync(path, updated);
        }
        console.log(`Updated all ${packages} package versions and Cargo lockfile entries to ${version}.`);
    }
} catch (error) {
    console.error(error.message);
    process.exitCode = 1;
}
