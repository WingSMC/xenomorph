import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'node:path';
import { defineConfig } from 'vite';

// Bundles the Svelte webviews into out/webview/webview.{js,css}.
export default defineConfig({
    plugins: [svelte()],
    define: { 'process.env.NODE_ENV': JSON.stringify('production') },
    build: {
        outDir: 'out/webview',
        emptyOutDir: true,
        minify: true,
        target: 'es2022',
        lib: {
            entry: resolve(__dirname, 'src/webview/main.ts'),
            formats: ['es'],
            fileName: () => 'webview.js',
            cssFileName: 'webview',
        },
    },
});
