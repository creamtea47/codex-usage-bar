import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'es2022',
  },
  test: {
    environment: 'jsdom',
    globals: true,
    // Intel macOS GitHub runners can take more than Vitest's 5 s default
    // while React/MUI initializes; keep enough headroom without hiding hangs.
    testTimeout: 15_000,
  },
});
