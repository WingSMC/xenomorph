import { builtinModules } from 'node:module';
import { resolve } from 'node:path';
import { defineConfig } from 'vite';

// Bundles the extension host code (Node, CommonJS). The webview is built
// separately by vite.webview.config.ts.
export default defineConfig({
    resolve: {
        conditions: ['node'],
        mainFields: ['module', 'main'],
    },
    build: {
        outDir: 'out',
        emptyOutDir: false,
        minify: true,
        target: 'node18',
        lib: {
            entry: resolve(__dirname, 'src/extension/extension.ts'),
            formats: ['cjs'],
            fileName: () => 'extension.js',
        },
        rollupOptions: {
            external: [
                'vscode',
                ...builtinModules,
                ...builtinModules.map((name) => `node:${name}`),
            ],
            output: { codeSplitting: false },
        },
    },
});
