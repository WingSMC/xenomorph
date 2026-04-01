import { resolve } from 'node:path';
import { defineConfig } from 'vite';

export default defineConfig({
    build: {
        lib: {
            entry: resolve(__dirname, 'src/extension.ts'),
            formats: ['cjs'],
            fileName: () => 'extension.js',
        },
        outDir: 'out',
        sourcemap: true,
        minify: false,
        rollupOptions: {
            external: [
                'vscode',
                /^vscode-languageclient/,
                /^vscode-languageserver/,
                /^vscode-jsonrpc/,
                /^node:/,
            ],
            output: {
                manualChunks: undefined,
            },
        },
        target: 'node18',
        emptyOutDir: true,
    },
    resolve: {
        extensions: ['.ts', '.js'],
        conditions: ['node'],
    },
});
