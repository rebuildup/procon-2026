import { defineConfig, loadEnv } from 'vite';
import solid from 'vite-plugin-solid';
import { fileURLToPath } from 'node:url';

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '');
  return {
    plugins: [solid()],
    resolve: {
      alias: {
        '@': fileURLToPath(new URL('./src', import.meta.url)),
      },
    },
    server: {
      proxy: {
        '/api/match': {
          target: env['MATCH_URL'] || 'http://127.0.0.1:3000',
          changeOrigin: true,
          rewrite: (path) => path.replace(/^\/api\/match/, ''),
          headers: env['PROCON_TOKEN'] ? { 'Procon-Token': env['PROCON_TOKEN'] } : {},
        },
      },
      port: 5173,
      strictPort: false,
      // Avoid leaking the host: the dev server must not bind 0.0.0.0 by default.
      host: '127.0.0.1',
    },
    build: {
      target: 'esnext',
      sourcemap: true,
    },
    worker: {
      format: 'es',
    },
  };
});
