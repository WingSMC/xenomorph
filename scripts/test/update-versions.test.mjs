import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('../update-versions.mjs', import.meta.url));

function fixture(t) {
    const prefix = join(tmpdir(), 'xenomorph-versions-');
    const root = mkdtempSync(prefix);
    t.after(() => {
        assert.ok(resolve(root).startsWith(resolve(prefix)));
        rmSync(root, { recursive: true, force: true });
    });
    function write(path, content) {
        const destination = join(root, path);
        mkdirSync(dirname(destination), { recursive: true });
        writeFileSync(destination, content);
    }
    write('scripts/update-versions.mjs', '');
    copyFileSync(script, join(root, 'scripts/update-versions.mjs'));
    write('package.json', '{\n    "name": "workspace",\n    "scripts": {}\n}\n');
    write('extensions/vscode/package.json', '{\n  "name": "extension",\n  "version": "0.1.2",\n  "dependencies": {"external": "^4.0.0"}\n}\n');
    write('Cargo.toml', '[workspace]\nmembers = ["plugin"]\n');
    write('plugin/Cargo.toml', '[package]\nname = "plugin"\nversion = "0.1.2" # release version\n\n[dependencies]\nexternal = { version = "4.0.0" }\n');
    write('Cargo.lock', 'version = 4\n\n[[package]]\nname = "plugin"\nversion = "0.1.2"\ndependencies = ["external"]\n\n[[package]]\nname = "external"\nversion = "4.0.0"\nsource = "registry+https://example.org"\n');
    return {
        root,
        write,
        read: (path) => readFileSync(join(root, path), 'utf8'),
        run: (...args) => spawnSync(process.execPath, [join(root, 'scripts/update-versions.mjs'), ...args], {
            cwd: join(root, 'extensions/vscode'),
            encoding: 'utf8',
        }),
    };
}

test('updates manifests and local lock entries from any working directory, preserving dependencies', (t) => {
    const f = fixture(t);
    const result = f.run('1.2.3');
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(f.read('package.json')).version, '1.2.3');
    const extension = JSON.parse(f.read('extensions/vscode/package.json'));
    assert.equal(extension.version, '1.2.3');
    assert.deepEqual(extension.dependencies, { external: '^4.0.0' });
    assert.match(f.read('plugin/Cargo.toml'), /version = "1\.2\.3" # release version/);
    assert.match(f.read('plugin/Cargo.toml'), /external = \{ version = "4\.0\.0" \}/);
    assert.match(f.read('Cargo.lock'), /name = "plugin"\nversion = "1\.2\.3"/);
    assert.match(f.read('Cargo.lock'), /name = "external"\nversion = "4\.0\.0"/);
    assert.equal(f.read('Cargo.toml'), '[workspace]\nmembers = ["plugin"]\n');
    assert.equal(f.run('--check', '1.2.3').status, 0);
    const firstUpdate = f.read('plugin/Cargo.toml');
    assert.equal(f.run('1.2.3').status, 0);
    assert.equal(f.read('plugin/Cargo.toml'), firstUpdate);
});

test('check mode reports manifest and lockfile mismatches without writing', (t) => {
    const f = fixture(t);
    const before = f.read('package.json');
    const result = f.run('--check', '1.2.3');
    assert.equal(result.status, 1);
    assert.match(result.stderr, /package\.json/);
    assert.match(result.stderr, /Cargo\.lock \(plugin\)/);
    assert.equal(f.read('package.json'), before);
    assert.equal(f.run('1.2.3').status, 0);
    f.write('Cargo.lock', f.read('Cargo.lock').replace('version = "1.2.3"', 'version = "0.1.2"'));
    assert.equal(f.run('--check', '1.2.3').status, 1);
});

test('rejects invalid versions and argument counts before writing', (t) => {
    const f = fixture(t);
    const before = f.read('package.json');
    for (const args of [[], ['1.2'], ['v1.2.3'], ['01.2.3'], ['1.2.3-01'], ['1.2.3', 'extra'], ['--check']]) {
        const result = f.run(...args);
        assert.equal(result.status, 1, args.join(' '));
        assert.match(result.stderr, /Usage:/);
        assert.equal(f.read('package.json'), before);
    }
    assert.equal(f.run('1.2.3-rc.1+build.42').status, 0);
    assert.equal(f.run('--check', '1.2.3-rc.1+build.42').status, 0);
});

test('skips generated dependencies and nested repositories', (t) => {
    const f = fixture(t);
    const ignored = ['node_modules/dependency/package.json', 'target/Cargo.toml', '.hidden/package.json', 'schema/submodule/package.json'];
    for (const path of ignored) f.write(path, 'invalid manifest that must not be read');
    f.write('schema/submodule/.git', 'gitdir: ../../.git/modules/schema/submodule');
    assert.equal(f.run('1.2.3').status, 0);
    assert.equal(f.run('--check', '1.2.3').status, 0);
    for (const path of ignored) assert.equal(f.read(path), 'invalid manifest that must not be read');
});

test('a malformed manifest causes no partial update', (t) => {
    const f = fixture(t);
    f.write('z-broken/package.json', '{');
    const before = f.read('package.json');
    const result = f.run('1.2.3');
    assert.equal(result.status, 1);
    assert.equal(f.read('package.json'), before);
    assert.match(f.read('plugin/Cargo.toml'), /version = "0\.1\.2"/);
});
