import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath } from 'node:url'
// A JSON module provides only a default export, and the import attribute is
// what Vite's native config loader (the default in a future major) requires.
import manifest from './package.json' with { type: 'json' }
import { fillSplashPage, splashValues } from './src/splash-page.ts'

// Tauri serves the frontend from a fixed port and expects a static build in dist/.
export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    // The splash in index.html paints before the bundle - and `define` below -
    // has loaded, so its colours, mark, words and version are written into the
    // page itself, in dev and in the build alike. `pre`, because the build
    // lifts the inline <style> out and hands it to Tailwind as CSS, which
    // cannot read `{{dark.bg}}` as a colour.
    {
      name: 'midda:splash-page',
      transformIndexHtml: {
        order: 'pre',
        handler: (html) => fillSplashPage(html, splashValues(manifest.version)),
      },
    },
  ],
  define: {
    __APP_VERSION__: JSON.stringify(manifest.version),
  },
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/crates/**'] },
  },
})
