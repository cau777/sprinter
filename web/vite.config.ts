import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";
import { cpSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const webRoot = fileURLToPath(new URL(".", import.meta.url));
const pdfjsRoot = resolve(webRoot, "../node_modules/pdfjs-dist");
const pdfPublicRoot = resolve(webRoot, "public/pdfjs");

function copyPdfAssets() {
  mkdirSync(pdfPublicRoot, { recursive: true });
  cpSync(resolve(pdfjsRoot, "cmaps"), resolve(pdfPublicRoot, "cmaps"), { recursive: true });
  cpSync(resolve(pdfjsRoot, "standard_fonts"), resolve(pdfPublicRoot, "standard_fonts"), { recursive: true });
  cpSync(resolve(pdfjsRoot, "legacy/build/pdf.worker.min.mjs"), resolve(pdfPublicRoot, "pdf.worker.min.mjs"));
}

const pdfAssets = {
  name: "sprinter-pdf-assets",
  buildStart: copyPdfAssets,
  configureServer: copyPdfAssets,
};

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    pdfAssets,
    VitePWA({
      registerType: "prompt",
      includeAssets: ["sprinter.svg", "sprinter-maskable.svg"],
      manifest: {
        name: "Sprinter",
        short_name: "Sprinter",
        description: "A private, self-hosted AI chat workspace.",
        theme_color: "#080b12",
        background_color: "#080b12",
        display: "standalone",
        start_url: "/",
        icons: [
          { src: "/sprinter.svg", sizes: "any", type: "image/svg+xml", purpose: "any" },
          { src: "/sprinter-maskable.svg", sizes: "any", type: "image/svg+xml", purpose: "maskable" },
        ],
      },
      workbox: {
        globPatterns: ["**/*.{js,css,html,svg,woff2}"],
        globIgnores: ["**/pdfjs/**", "**/pdfjs-*.js"],
        runtimeCaching: [{
          urlPattern: ({ url }) => url.pathname.startsWith("/pdfjs/") || /\/pdfjs-[^/]+\.js$/.test(url.pathname),
          handler: "CacheFirst",
          options: {
            cacheName: "sprinter-pdf-assets",
            expiration: { maxEntries: 320, maxAgeSeconds: 60 * 60 * 24 * 365 },
            cacheableResponse: { statuses: [0, 200] },
          },
        }],
        navigateFallback: "index.html",
        cleanupOutdatedCaches: true,
      },
    }),
  ],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("/pdfjs-dist/")) return "pdfjs";
        },
      },
    },
  },
  server: { port: 5173 },
});
