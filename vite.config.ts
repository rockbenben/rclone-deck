import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// strictPort：端口被占时报错而不是悄悄换端口，与 rcd 的"端口被占就报错"策略一致。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022', outDir: 'dist', emptyOutDir: true },
});
