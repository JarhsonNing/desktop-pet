import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
const local = (path: string) => fileURLToPath(new URL(path, import.meta.url));
export default defineConfig({ plugins: [vue(), tailwindcss()], base: './', resolve: { alias: [
  { find: /^@earendil-works\/pi-ai$/, replacement: local('./node_modules/@earendil-works/pi-ai/dist/models.js') },
] }, build: { target: 'es2022', rollupOptions: { input: ['index.html', 'agent.html'] } } });
