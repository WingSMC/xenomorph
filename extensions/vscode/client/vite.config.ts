import { builtinModules } from 'node:module';
import { resolve } from 'node:path';
import { defineConfig } from 'vite';

const external = [
    'vscode',
    ...builtinModules,
    ...builtinModules.map((m) => `node:${m}`),
];

export default defineConfig({
    resolve: {
        extensions: ['.ts', '.js'],
        conditions: ['node'],
        mainFields: ['module', 'main'],
    },
    build: {
        outDir: 'out',
        sourcemap: true,
        minify: true,
        emptyOutDir: true,
        target: 'node18',
        lib: {
            entry: resolve(__dirname, 'src/extension.ts'),
            formats: ['cjs'],
            fileName: () => 'extension.js',
        },
        rollupOptions: {
            external,
            output: {
                codeSplitting: false,
            },
        },
    },
});
