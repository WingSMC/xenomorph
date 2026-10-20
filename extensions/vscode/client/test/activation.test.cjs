const assert = require('node:assert/strict');
const fs = require('node:fs');
const Module = require('node:module');
const path = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');
const ts = require('typescript');

const reloadCommand = 'xenomorph.reloadPlugins';

test('activation and LSP restart register the reload command without collisions', async () => {
    const handlers = new Map();
    const logs = [];
    const errors = [];
    const requests = [];
    const subscriptions = [];
    const channel = {
        appendLine: (line) => logs.push(line),
        dispose() {},
    };
    const vscode = {
        CancellationError: class extends Error {},
        EventEmitter: class {
            event = () => ({ dispose() {} });
            fire() {}
            dispose() {}
        },
        commands: {
            registerCommand(command, handler) {
                if (handlers.has(command)) {
                    throw new Error(`command '${command}' already exists`);
                }
                handlers.set(command, handler);
                return { dispose: () => handlers.delete(command) };
            },
        },
        languages: {
            registerCodeLensProvider: () => ({ dispose() {} }),
        },
        window: {
            createOutputChannel: () => channel,
            showErrorMessage: async (message) => errors.push(message),
        },
        workspace: {
            getConfiguration: () => ({ get: (_key, fallback) => fallback }),
            onDidChangeConfiguration: () => ({ dispose() {} }),
        },
    };

    // Use the dependency's real initialization and command dispatch; only the
    // VS Code host and language-server transport are replaced.
    const originalLoad = Module._load;
    let ExecuteCommandFeature;
    try {
        Module._load = function (id, ...args) {
            return id === 'vscode' ? vscode : originalLoad.call(this, id, ...args);
        };
        ({ ExecuteCommandFeature } = require('vscode-languageclient/lib/common/executeCommand'));
    } finally {
        Module._load = originalLoad;
    }

    let languageClient;
    class LanguageClient {
        constructor(_id, _name, _serverOptions, clientOptions) {
            this.middleware = clientOptions.middleware ?? {};
            this.feature = new ExecuteCommandFeature(this);
            this.running = false;
            languageClient = this;
        }
        async start() {
            this.feature.initialize({
                executeCommandProvider: { commands: [reloadCommand] },
            });
            this.running = true;
        }
        isRunning() {
            return this.running;
        }
        async sendRequest(type, params) {
            requests.push({
                method: typeof type === 'string' ? type : type.method,
                // JSON-RPC serializes parameters across the VM boundary.
                params: JSON.parse(JSON.stringify(params)),
            });
            if (this.requestError) {
                throw this.requestError;
            }
            return { reloaded: true };
        }
        handleFailedRequest(_type, _token, error) {
            throw error;
        }
        async restart() {
            await this.stop();
            await this.start();
        }
        async stop() {
            this.feature.clear();
            this.running = false;
        }
    }

    const modules = new Map();
    function loadSource(filename) {
        if (modules.has(filename)) {
            return modules.get(filename).exports;
        }
        const sourceModule = { exports: {} };
        modules.set(filename, sourceModule);
        const { outputText } = ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
            compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
            fileName: filename,
        });
        vm.runInNewContext(outputText, {
            module: sourceModule,
            exports: sourceModule.exports,
            require(id) {
                if (id === 'vscode') return vscode;
                if (id === 'vscode-languageclient/node') return { LanguageClient };
                if (id.startsWith('.')) {
                    return loadSource(path.resolve(path.dirname(filename), `${id}.ts`));
                }
                return require(id);
            },
        }, { filename });
        return sourceModule.exports;
    }

    const extension = loadSource(path.resolve(__dirname, '../src/extension/extension.ts'));
    try {
        extension.activate({ subscriptions });
        await new Promise(setImmediate);
        assert.equal(languageClient.isRunning(), true, errors.join('\n'));
        assert.ok(handlers.has(reloadCommand));

        await handlers.get(reloadCommand)();
        assert.deepEqual(requests, [{
            method: 'workspace/executeCommand',
            params: { command: reloadCommand, arguments: [] },
        }]);
        const successMessage = '[LSP] Plugins reloaded without restarting the server.';
        assert.ok(logs.includes(successMessage));

        await handlers.get('xenomorph.restartLsp')();
        assert.equal(languageClient.isRunning(), true);
        assert.ok(handlers.has(reloadCommand));
        await handlers.get(reloadCommand)();
        assert.equal(requests.length, 2);
        assert.deepEqual(errors, []);

        languageClient.requestError = new Error('Invalid plugin configuration');
        await handlers.get(reloadCommand)();
        assert.equal(errors.length, 1);
        assert.match(errors[0], /Xenomorph plugin reload failed.*Invalid plugin configuration/);
        assert.ok(logs.some((line) => /Plugin reload failed.*Invalid plugin configuration/.test(line)));
        assert.equal(logs.filter((line) => line === successMessage).length, 2);
        assert.equal(languageClient.isRunning(), true);
    } finally {
        await extension.deactivate();
        subscriptions.forEach((subscription) => subscription.dispose());
    }
    assert.equal(handlers.size, 0);
});
