<template>
  <!--
    Two-column layout: a fixed left nav rail on md+ screens, collapsing
    to a top bar on small screens. The right column holds the routed
    page content + a thin status bar that reflects the WebSocket
    connection state from `useDashboardFeed`.
  -->
  <div class="flex min-h-screen bg-surface-0 text-slate-100">
    <!-- Left navigation -->
    <aside
      class="hidden md:flex md:w-60 md:flex-col md:border-r md:border-slate-800 md:bg-surface-1"
    >
      <div class="border-b border-slate-800 px-5 py-4">
        <div class="text-xs uppercase tracking-wide text-slate-400">
          lodestone
        </div>
        <div class="mt-0.5 font-semibold">dashboard</div>
      </div>
      <nav class="flex-1 space-y-0.5 px-2 py-3">
        <NuxtLink
          v-for="item in navItems"
          :key="item.to"
          :to="item.to"
          class="block rounded px-3 py-2 text-sm text-slate-300 transition hover:bg-surface-2 hover:text-white"
          active-class="bg-surface-2 text-white"
        >
          {{ item.label }}
        </NuxtLink>
      </nav>
      <div class="border-t border-slate-800 px-5 py-3 text-xs text-slate-500">
        v{{ feedVersion }} · uptime {{ feedUptime }}
      </div>
    </aside>

    <!-- Mobile top bar -->
    <header
      class="fixed inset-x-0 top-0 z-20 flex items-center justify-between border-b border-slate-800 bg-surface-1 px-4 py-3 md:hidden"
    >
      <div class="font-semibold">lodestone</div>
      <span
        class="rounded px-2 py-0.5 text-xs"
        :class="statusBadge"
        >{{ status }}</span
      >
    </header>

    <!-- Main column -->
    <main class="flex w-full flex-1 flex-col pt-14 md:pt-0">
      <!-- Status bar (desktop) -->
      <div
        class="hidden items-center justify-between border-b border-slate-800 bg-surface-1/60 px-6 py-2 text-xs md:flex"
      >
        <div class="flex items-center gap-2">
          <span class="h-2 w-2 rounded-full" :class="statusDot" />
          <span class="text-slate-400">
            {{ status }}<span v-if="lastError"> — {{ lastError }}</span>
          </span>
        </div>
        <div class="text-slate-500">
          last update: {{ lastUpdatedLabel }}
        </div>
      </div>

      <div class="flex-1 px-4 py-6 md:px-8 md:py-8">
        <NuxtPage />
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'

const navItems = [
  { to: '/', label: 'Overview' },
  { to: '/tools', label: 'Tools' },
  { to: '/memory', label: 'Memory' },
  { to: '/constellation', label: 'Constellation' },
]

// Provide the feed at the layout level so child pages can `inject()` it
// rather than each one opening its own socket.
const { snapshot, status, lastError, lastUpdatedAt } = useDashboardFeed()
provide('dashboardFeed', { snapshot, status, lastError, lastUpdatedAt })

const feedVersion = computed(() => snapshot.value?.server.version ?? '—')
const feedUptime = computed(() => {
  const s = snapshot.value?.server.uptime_secs ?? 0
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86400) return `${Math.floor(s / 3600)}h`
  return `${Math.floor(s / 86400)}d`
})

const statusDot = computed(() => ({
  'bg-accent-ok': status.value === 'open',
  'bg-accent-warn': status.value === 'reconnecting' || status.value === 'connecting',
  'bg-accent-err': status.value === 'closed',
}))
const statusBadge = computed(() => ({
  'bg-accent-ok/20 text-accent-ok': status.value === 'open',
  'bg-accent-warn/20 text-accent-warn':
    status.value === 'reconnecting' || status.value === 'connecting',
  'bg-accent-err/20 text-accent-err': status.value === 'closed',
}))
const lastUpdatedLabel = computed(() =>
  lastUpdatedAt.value
    ? lastUpdatedAt.value.toLocaleTimeString()
    : '— (waiting for first snapshot)',
)
</script>
