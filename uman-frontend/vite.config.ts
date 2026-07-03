import path from "path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    // Proxy API and v1 routes to the Rust backend in dev mode
    proxy: {
      "/api": "http://localhost:8084",
      "/v1": "http://localhost:8084",
      "/messages": "http://localhost:8084",
      "/healthz": "http://localhost:8084",
    },
  },
})
