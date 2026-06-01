<!--
  Constellation page — node identity, peer table, delegation knobs, seed
  accounting. The peer table is the load-bearing view: one row per known
  peer with reachability + reputation + advertised delegation flag.
-->
<template>
  <div v-if="!snapshot" class="text-slate-400">Waiting for snapshot…</div>
  <div v-else-if="!snapshot.constellation.enabled" class="text-sm text-slate-500">
    Constellation disabled — set
    <span class="font-mono">[network].enabled = true</span>
    and add at least one peer (or turn on
    <span class="font-mono">[network].mdns</span>) to participate.
  </div>
  <div v-else class="space-y-8">
    <PageHeader title="Constellation" @open-settings="settingsOpen = true" />

    <SettingsDrawer
      :open="settingsOpen"
      subsystem="Constellation"
      @close="settingsOpen = false"
    >
      <form class="space-y-5" @submit.prevent>
        <div>
          <label class="flex items-center justify-between gap-3 text-sm">
            <span>
              <span class="font-medium text-slate-100">Delegation</span>
              <span class="block text-xs text-slate-400">
                Accept fetches peers ask us to perform.
              </span>
            </span>
            <input
              type="checkbox"
              class="h-5 w-5 accent-accent-info"
              :checked="form.delegation_enabled"
              @change="patch({ delegation_enabled: ($event.target as HTMLInputElement).checked })"
            />
          </label>
        </div>

        <div>
          <label class="block text-sm">
            <span class="font-medium text-slate-100">Max peers</span>
            <span class="block text-xs text-slate-400">
              Cap on peers consulted per query. 1–256.
            </span>
            <input
              type="number"
              min="1"
              max="256"
              class="mt-2 w-24 rounded border border-slate-700 bg-surface-0 px-2 py-1 text-sm"
              :value="form.max_peers"
              @change="patch({ max_peers: Number(($event.target as HTMLInputElement).value) })"
            />
          </label>
        </div>

        <div>
          <label class="block text-sm">
            <span class="font-medium text-slate-100">Min agreement</span>
            <span class="block text-xs text-slate-400">
              Peers that must corroborate a result before returning without a local search. 1–16.
            </span>
            <input
              type="number"
              min="1"
              max="16"
              class="mt-2 w-24 rounded border border-slate-700 bg-surface-0 px-2 py-1 text-sm"
              :value="form.min_agreement"
              @change="patch({ min_agreement: Number(($event.target as HTMLInputElement).value) })"
            />
          </label>
        </div>

        <div v-if="patchError" class="rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err">
          {{ patchError }}
        </div>

        <hr class="border-slate-800" />

        <div class="space-y-3 text-sm">
          <div class="text-xs uppercase tracking-wide text-slate-500">
            Restart required
          </div>
          <ReadOnlyRow label="mDNS" :value="snapshot.constellation.mdns_configured ? 'on' : 'off'" />
          <ReadOnlyRow label="Sync interval" :value="`${snapshot.constellation.sync_secs_configured}s`" />
          <ReadOnlyRow label="Request timeout" :value="`${snapshot.constellation.request_timeout_ms_configured}ms`" />
        </div>
      </form>
    </SettingsDrawer>

    <section>
      <SectionHeading>Identity</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-3">
        <StatCard
          label="Node id"
          :value="snapshot.constellation.node_id"
          sub="stable per host"
        />
        <StatCard
          label="Constellation id"
          :value="snapshot.constellation.constellation_id"
          sub="merged with peers via gossip"
        />
        <StatCard
          label="Peers"
          :value="snapshot.constellation.peer_count"
          :sub="`${reachableCount} reachable, ${snapshot.constellation.peer_count - reachableCount} pending`"
        />
      </div>
    </section>

    <section>
      <SectionHeading>Topology</SectionHeading>
      <ConstellationGraph
        :node-id="snapshot.constellation.node_id"
        :peers="snapshot.constellation.peers"
        :local-urls="snapshot.constellation.local_urls"
      />
    </section>

    <section>
      <SectionHeading>Peers</SectionHeading>
      <div
        class="overflow-hidden rounded-lg border border-slate-800 bg-surface-1"
      >
        <table class="w-full text-sm">
          <thead class="bg-surface-2 text-xs uppercase text-slate-400">
            <tr>
              <th class="px-4 py-2 text-left">URL</th>
              <th class="px-4 py-2 text-left">Node id</th>
              <th class="px-4 py-2 text-right">Reputation</th>
              <th class="px-4 py-2 text-center">Reachable</th>
              <th class="px-4 py-2 text-center">Delegation</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-800">
            <tr
              v-for="p in snapshot.constellation.peers"
              :key="p.url"
              class="hover:bg-surface-2/60"
            >
              <td class="px-4 py-2 font-mono text-xs">{{ p.url }}</td>
              <td class="px-4 py-2 font-mono text-xs text-slate-400">
                {{ p.node_id ?? '—' }}
              </td>
              <td class="px-4 py-2 text-right font-mono">
                {{ p.reputation.toFixed(2) }}
              </td>
              <td class="px-4 py-2 text-center">
                <span
                  class="inline-block h-2 w-2 rounded-full"
                  :class="p.reachable ? 'bg-accent-ok' : 'bg-accent-warn'"
                />
              </td>
              <td class="px-4 py-2 text-center">
                <span
                  class="rounded px-2 py-0.5 text-xs"
                  :class="
                    p.delegation_enabled
                      ? 'bg-accent-ok/20 text-accent-ok'
                      : 'bg-surface-2 text-slate-500'
                  "
                >
                  {{ p.delegation_enabled ? 'on' : 'off' }}
                </span>
              </td>
            </tr>
            <tr v-if="snapshot.constellation.peers.length === 0">
              <td colspan="5" class="px-4 py-6 text-center text-slate-500">
                No peers yet — mDNS may still be discovering, or no
                <span class="font-mono">[network].peers</span> are
                configured.
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <section>
      <SectionHeading>Delegation</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard
          label="Status"
          :value="snapshot.constellation.delegation_enabled ? 'enabled' : 'disabled'"
        />
        <StatCard
          label="Jobs / peer / hr"
          :value="snapshot.constellation.delegation_max_jobs_per_peer_per_hour"
        />
        <StatCard
          label="Bytes / job"
          :value="fmtBytes(snapshot.constellation.delegation_max_bytes_per_job)"
        />
        <StatCard
          label="Total bytes / hr"
          :value="fmtBytes(snapshot.constellation.delegation_total_bytes_per_hour)"
        />
      </div>
    </section>

    <section>
      <SectionHeading>Seeds</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-3">
        <StatCard
          label="Served"
          :value="fmtBytes(snapshot.constellation.total_served_bytes)"
          sub="bytes given to peers"
        />
        <StatCard
          label="Fetched"
          :value="fmtBytes(snapshot.constellation.total_fetched_bytes)"
          sub="bytes pulled from peers"
        />
        <StatCard
          label="Ratio"
          :value="seedRatio"
          sub="served / fetched"
        />
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, inject, reactive, ref, watchEffect, type Ref } from 'vue'
import type { Snapshot } from '~/types/ws'

const feed = inject<{ snapshot: Ref<Snapshot | null> }>('dashboardFeed')!
const snapshot = feed.snapshot

const reachableCount = computed(
  () => snapshot.value?.constellation.peers.filter((p) => p.reachable).length ?? 0,
)
const seedRatio = computed(() => {
  const s = snapshot.value?.constellation
  if (!s || s.total_fetched_bytes === 0) return '—'
  return (s.total_served_bytes / s.total_fetched_bytes).toFixed(2)
})

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`
  return `${(n / 1024 / 1024 / 1024).toFixed(1)} GB`
}

// Settings drawer state. `form` mirrors the snapshot's runtime values
// so the inputs reflect what the server actually accepted (the backend
// clamps and echoes back); we mirror from snapshot each tick.
const settingsOpen = ref(false)
const form = reactive({
  delegation_enabled: false,
  max_peers: 16,
  min_agreement: 2,
})
watchEffect(() => {
  const c = snapshot.value?.constellation
  if (!c) return
  form.delegation_enabled = c.delegation_enabled
  form.max_peers = c.max_peers
  form.min_agreement = c.min_agreement
})

const { patch: patchSettings } = useSettingsApi()
const patchError = ref<string | null>(null)
async function patch(body: Record<string, unknown>) {
  patchError.value = null
  try {
    await patchSettings('constellation', body)
    // The next WS snapshot will reflect the applied state, so we don't
    // need to merge the response into `form` ourselves.
  } catch (e) {
    patchError.value = e instanceof Error ? e.message : String(e)
  }
}
</script>
