import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// In dev, the API runs on :8080 (cargo run --bin api).
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': 'http://localhost:8080',
      '/ws': { target: 'ws://localhost:8080', ws: true },
    },
  },
});
