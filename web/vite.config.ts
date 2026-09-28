import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    VitePWA({
      registerType: "prompt",
      includeAssets: ["sprinter.svg"],
      manifest: {
        name: "Sprinter",
        short_name: "Sprinter",
        description: "A private, self-hosted AI chat workspace.",
        theme_color: "#080b12",
        background_color: "#080b12",
        display: "standalone",
        start_url: "/",
        icons: [
          { src: "/sprinter.svg", sizes: "any", type: "image/svg+xml", purpose: "any maskable" },
        ],
      },
      workbox: {
        globPatterns: ["**/*.{js,css,html,svg,woff2}"],
        navigateFallback: "index.html",
        cleanupOutdatedCaches: true,
      },
    }),
  ],
  build: { outDir: "dist", emptyOutDir: true },
  server: { port: 5173 },
});
