// Nuxt 3 configuration for the lodestone-mcp dashboard.
//
// One responsive single-page app with a left-side navigation rail
// (Tailwind), connected to the backend `/ws/status` WebSocket feed for
// live server, memory, and constellation snapshots.
//
// Runtime config:
//   wsUrl — the WebSocket URL the dashboard connects to. Defaults to a
//           same-origin `ws(s)://<host>/ws/status` resolution computed at
//           runtime in the dashboard composable. Override via
//           NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status when running
//           the dev server against a remote lodestone-mcp.
//   token  — passed as ?token=… on connect; matches [network].token on
//            the backend. Empty when the backend doesn't require one.

export default defineNuxtConfig({
  compatibilityDate: '2025-01-01',
  devtools: { enabled: true },
  modules: ['@nuxtjs/tailwindcss', '@vueuse/nuxt'],
  ssr: false,
  app: {
    // The Rust binary serves the SPA under `/dashboard/`, so every
    // generated asset URL (`<link rel="stylesheet" href="…">`,
    // `<script src="…">`, route paths) must carry the same prefix.
    // Without this, the SPA HTML loads but every `/_nuxt/…` reference
    // resolves to a 404 against the bare backend (`/` → MCP only) and
    // the page hangs with no styles + no JS.
    baseURL: '/dashboard/',
    buildAssetsDir: '/_nuxt/',
    head: {
      title: 'lodestone-mcp dashboard',
      meta: [
        { charset: 'utf-8' },
        { name: 'viewport', content: 'width=device-width, initial-scale=1' },
        {
          name: 'description',
          content:
            'Live dashboard for a lodestone-mcp server: status, memory, constellation.',
        },
      ],
    },
  },
  runtimeConfig: {
    public: {
      // Override at runtime with NUXT_PUBLIC_WS_URL when the dashboard is
      // not co-served with the backend. Empty string = same-origin (the
      // composable derives ws(s)://host/ws/status from window.location).
      wsUrl: '',
      // NUXT_PUBLIC_WS_TOKEN — only when [network].token is set.
      wsToken: '',
    },
  },
})
