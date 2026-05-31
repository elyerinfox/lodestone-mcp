// Tailwind config for the dashboard. The Nuxt module auto-detects
// content paths but we set them explicitly so a content-aware purge
// works the same in production builds.

import type { Config } from 'tailwindcss'

export default {
  content: [
    './components/**/*.{vue,js,ts}',
    './layouts/**/*.vue',
    './pages/**/*.vue',
    './plugins/**/*.{js,ts}',
    './app.vue',
    './error.vue',
  ],
  theme: {
    extend: {
      colors: {
        // The dashboard uses a neutral dark theme — Grafana-ish, so an
        // operational telemetry surface feels at home next to the
        // `chart_grafana` family.
        surface: {
          0: '#0f1115',
          1: '#161922',
          2: '#1d2230',
          3: '#252b3c',
        },
        accent: {
          ok: '#34d399',
          warn: '#fbbf24',
          err: '#f87171',
          info: '#60a5fa',
        },
      },
      fontFamily: {
        mono: [
          'ui-monospace',
          'SFMono-Regular',
          'Menlo',
          'Monaco',
          'Consolas',
          'monospace',
        ],
      },
    },
  },
  plugins: [],
} satisfies Config
