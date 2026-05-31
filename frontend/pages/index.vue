<!--
  Overview page — the landing dashboard. One row of "headline" stats, one
  list of active providers, one mini-readout of each subsystem (memory +
  constellation) deep-linked to its dedicated page.
-->
<template>
  <div v-if="!snapshot" class="text-slate-400">
    Waiting for the first snapshot from <span class="font-mono">/ws/status</span>…
  </div>
  <div v-else class="space-y-8">
    <section>
      <SectionHeading>Server</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard
          label="Version"
          :value="snapshot.server.version"
          :sub="snapshot.server.name"
        />
        <StatCard
          label="Uptime"
          :value="uptimeHuman"
          sub="since last boot"
        />
        <StatCard
          label="Tools active"
          :value="snapshot.server.tools_active"
          :sub="`${snapshot.server.tools_disabled} hidden by config`"
        />
        <StatCard
          label="Providers"
          :value="snapshot.server.providers.length"
          :sub="providerKindSummary"
        />
      </div>
    </section>

    <section>
      <SectionHeading>Memory</SectionHeading>
      <div v-if="!snapshot.memory.enabled" class="text-sm text-slate-500">
        Memory disabled — set <span class="font-mono">[memory].enabled = true</span>
        to populate this panel.
      </div>
      <div v-else class="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard label="Memos" :value="snapshot.memory.memos" />
        <StatCard label="Solutions" :value="snapshot.memory.solutions" />
        <StatCard label="Conversations" :value="snapshot.memory.conversations" />
        <StatCard label="Synonyms" :value="snapshot.memory.synonyms" />
      </div>
    </section>

    <section>
      <SectionHeading>Constellation</SectionHeading>
      <div v-if="!snapshot.constellation.enabled" class="text-sm text-slate-500">
        Constellation disabled — set
        <span class="font-mono">[network].enabled = true</span>
        to participate in a mesh.
      </div>
      <div v-else class="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard
          label="Node id"
          :value="snapshot.constellation.node_id"
          :sub="`mesh: ${snapshot.constellation.constellation_id}`"
        />
        <StatCard
          label="Peers"
          :value="snapshot.constellation.peer_count"
        />
        <StatCard
          label="Delegation"
          :value="snapshot.constellation.delegation_enabled ? 'on' : 'off'"
        />
        <StatCard
          label="Seed ratio"
          :value="seedRatio"
          sub="served / fetched bytes"
        />
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, inject, type Ref } from 'vue'
import type { Snapshot } from '~/types/ws'

const feed = inject<{ snapshot: Ref<Snapshot | null> }>('dashboardFeed')!
const snapshot = feed.snapshot

const uptimeHuman = computed(() => {
  const s = snapshot.value?.server.uptime_secs ?? 0
  const days = Math.floor(s / 86400)
  const hours = Math.floor((s % 86400) / 3600)
  const mins = Math.floor((s % 3600) / 60)
  if (days > 0) return `${days}d ${hours}h`
  if (hours > 0) return `${hours}h ${mins}m`
  if (mins > 0) return `${mins}m`
  return `${s}s`
})

const providerKindSummary = computed(() => {
  const ps = snapshot.value?.server.providers ?? []
  const byKind: Record<string, number> = {}
  for (const p of ps) byKind[p.kind] = (byKind[p.kind] ?? 0) + 1
  return Object.entries(byKind)
    .map(([k, n]) => `${k}:${n}`)
    .join('  ')
})

const seedRatio = computed(() => {
  const s = snapshot.value?.constellation
  if (!s) return '—'
  if (s.total_fetched_bytes === 0) return '—'
  return (s.total_served_bytes / s.total_fetched_bytes).toFixed(2)
})
</script>
