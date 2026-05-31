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
    <PageHeader title="Overview" @open-settings="settingsOpen = true" />

    <SettingsDrawer
      :open="settingsOpen"
      subsystem="Server"
      @close="settingsOpen = false"
    >
      <div class="space-y-5 text-sm">
        <div>
          <div class="text-xs uppercase tracking-wide text-slate-500 mb-2">
            Bind addresses
          </div>
          <div class="space-y-2">
            <ReadOnlyRow label="MCP" :value="snapshot.server.bind || '—'" />
            <ReadOnlyRow
              label="Constellation"
              :value="snapshot.server.constellation_bind || 'shares MCP port'"
            />
          </div>
        </div>

        <div>
          <div class="text-xs uppercase tracking-wide text-slate-500 mb-2">
            Secrets
          </div>
          <p class="mb-2 text-xs text-slate-400">
            Values are never sent over this socket — only whether each one is
            configured. Set via <span class="font-mono">config/</span> or the
            corresponding <span class="font-mono">LODESTONE_*</span> env var.
          </p>
          <div class="space-y-2">
            <SecretRow label="MCP auth token" :set="snapshot.server.secrets.auth_token" />
            <SecretRow label="Constellation token" :set="snapshot.server.secrets.network_token" />
            <SecretRow label="GitHub token" :set="snapshot.server.secrets.github_token" />
            <SecretRow label="NASA API key" :set="snapshot.server.secrets.nasa_key" />
            <SecretRow label="EIA API key" :set="snapshot.server.secrets.eia_key" />
          </div>
        </div>

        <hr class="border-slate-800" />

        <div>
          <div class="text-xs uppercase tracking-wide text-slate-500 mb-2">
            Log level
          </div>
          <p class="mb-2 text-xs text-slate-400">
            Reloads the tracing filter without restarting. Debug and trace
            also enable hyper at info so request-handling activity surfaces.
          </p>
          <select
            class="w-full rounded border border-slate-700 bg-surface-0 px-2 py-1.5 text-sm"
            :value="currentLogLevel"
            @change="setLogLevel(($event.target as HTMLSelectElement).value)"
          >
            <option value="error">error</option>
            <option value="warn">warn</option>
            <option value="info">info</option>
            <option value="debug">debug</option>
            <option value="trace">trace</option>
          </select>
          <div class="mt-2 text-xs font-mono text-slate-500">
            active: {{ snapshot.server.log_level }}
          </div>
          <div
            v-if="serverPatchError"
            class="mt-2 rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err"
          >
            {{ serverPatchError }}
          </div>
        </div>
      </div>
    </SettingsDrawer>

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
import { computed, inject, ref, type Ref } from 'vue'
import type { Snapshot } from '~/types/ws'

const feed = inject<{ snapshot: Ref<Snapshot | null> }>('dashboardFeed')!
const snapshot = feed.snapshot

const settingsOpen = ref(false)

const currentLogLevel = computed(() => {
  // Extract the level we'd expose in the dropdown from the live filter
  // string (e.g. "lodestone_mcp=info,rmcp=warn" → "info"). The set_level
  // call always builds the directive from one of the five keywords, so
  // splitting on the first '=' and ',' is reliable here.
  const s = snapshot.value?.server.log_level ?? ''
  const m = s.match(/=([a-z]+)/i)
  return m ? m[1].toLowerCase() : 'info'
})

const { patch: patchSettings } = useSettingsApi()
const serverPatchError = ref<string | null>(null)
async function setLogLevel(level: string) {
  serverPatchError.value = null
  try {
    await patchSettings('server', { log_level: level })
  } catch (e) {
    serverPatchError.value = e instanceof Error ? e.message : String(e)
  }
}

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
