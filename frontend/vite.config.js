import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:8000',
        changeOrigin: true,
      },
      '/api-docs': {
        target: 'http://127.0.0.1:8000',
        changeOrigin: true,
      },
      // Without this, `./start.sh` cannot complete a login: the dev server serves
      // index.html for /auth/login and the browser never reaches the backend's
      // redirect to the identity provider.
      '/auth': {
        target: 'http://127.0.0.1:8000',
        changeOrigin: true,
      },
    },
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: './src/test/setup.ts'
  }
})
