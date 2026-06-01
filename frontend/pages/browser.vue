<!--
  Browser sessions page — one row per open browser session with live
  URL + title + age + idle, plus a per-row "close" button that hits
  DELETE /api/browser/sessions/{id}. The settings drawer tunes the
  idle timeout and the concurrent cap; both apply immediately and a
  restart reverts to defaults.
-->
<template>
  <div v-if="!snapshot" class="text-slate-400">Waiting for snapshot…</div>
  <div v-else class="space-y-8">
    <PageHeader title="Browser" @open-settings="settingsOpen = true" />

    <SettingsDrawer
      :open="settingsOpen"
      subsystem="Browser"
      @close="settingsOpen = false"
    >
      <form class="space-y-5" @submit.prevent>
        <div>
          <label class="block text-sm">
            <span class="font-medium text-slate-100">Idle timeout (seconds)</span>
            <span class="block text-xs text-slate-400">
              Close a session that hasn't been touched for this long.
              30 – 86400. Default 1800 (30 min).
            </span>
            <input
              type="number"
              min="30"
              max="86400"
              class="mt-2 w-32 rounded border border-slate-700 bg-surface-0 px-2 py-1 text-sm"
              :value="snapshot.browser.idle_timeout_secs"
              @change="patch({ idle_timeout_secs: Number(($event.target as HTMLInputElement).value) })"
            />
          </label>
        </div>

        <div>
          <label class="block text-sm">
            <span class="font-medium text-slate-100">Max concurrent sessions</span>
            <span class="block text-xs text-slate-400">
              Cap on simultaneously open sessions. 1 – 64. Default 8.
              Past the cap, `browser_open` returns an error.
            </span>
            <input
              type="number"
              min="1"
              max="64"
              class="mt-2 w-24 rounded border border-slate-700 bg-surface-0 px-2 py-1 text-sm"
              :value="snapshot.browser.max_concurrent"
              @change="patch({ max_concurrent: Number(($event.target as HTMLInputElement).value) })"
            />
          </label>
        </div>

        <div v-if="patchError" class="rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err">
          {{ patchError }}
        </div>
      </form>
    </SettingsDrawer>

    <section>
      <SectionHeading>Active sessions</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-3">
        <StatCard
          label="Open"
          :value="snapshot.browser.sessions.length"
          :sub="`max ${snapshot.browser.max_concurrent}`"
        />
        <StatCard
          label="Idle timeout"
          :value="`${snapshot.browser.idle_timeout_secs}s`"
        />
        <StatCard
          label="Free slots"
          :value="snapshot.browser.max_concurrent - snapshot.browser.sessions.length"
        />
      </div>
    </section>

    <section v-if="snapshot.browser.pools.length > 0">
      <SectionHeading>Named pools</SectionHeading>
      <p class="mb-3 text-xs text-slate-400">
        Long-lived warm sessions for one site or vendor. The pool state
        machine flags poisoned sessions (CAPTCHA / 403 challenge / 429)
        as <span class="font-mono text-amber-300">suspect</span> on first
        warning and <span class="font-mono text-accent-err">blocked</span>
        on second. Reset disposes the session and starts fresh.
      </p>
      <div class="overflow-hidden rounded-lg border border-slate-800 bg-surface-1">
        <table class="w-full text-sm">
          <thead class="bg-surface-2 text-xs uppercase text-slate-400">
            <tr>
              <th class="px-4 py-2 text-left">Pool</th>
              <th class="px-4 py-2 text-left">State</th>
              <th class="px-4 py-2 text-left">URL</th>
              <th class="px-4 py-2 text-left">Last warning</th>
              <th class="px-4 py-2 text-right">Age</th>
              <th class="px-4 py-2 text-center"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-800">
            <tr
              v-for="p in snapshot.browser.pools"
              :key="p.name"
              class="hover:bg-surface-2/60"
            >
              <td class="px-4 py-2 font-mono text-xs">{{ p.name }}</td>
              <td class="px-4 py-2">
                <span
                  class="rounded px-2 py-0.5 text-xs"
                  :class="
                    p.state === 'healthy'
                      ? 'bg-accent-ok/20 text-accent-ok'
                      : p.state === 'suspect'
                      ? 'bg-amber-700/20 text-amber-300'
                      : 'bg-accent-err/20 text-accent-err'
                  "
                >{{ p.state }}</span>
              </td>
              <td class="px-4 py-2 font-mono text-xs text-slate-400 truncate max-w-md">
                {{ p.url || '—' }}
              </td>
              <td class="px-4 py-2 text-xs text-slate-400 truncate max-w-xs">
                {{ p.last_warning || '—' }}
              </td>
              <td class="px-4 py-2 text-right font-mono text-xs text-slate-400">
                {{ fmtSecs(p.age_secs) }}
              </td>
              <td class="px-4 py-2 text-center">
                <button
                  type="button"
                  class="rounded bg-accent-info/20 px-2 py-0.5 text-xs text-accent-info hover:bg-accent-info/30"
                  @click="resetPool(p.name)"
                >
                  reset
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-if="resetError" class="mt-3 rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err">
        {{ resetError }}
      </div>
    </section>

    <section>
      <SectionHeading>Sessions</SectionHeading>
      <div class="overflow-hidden rounded-lg border border-slate-800 bg-surface-1">
        <table class="w-full text-sm">
          <thead class="bg-surface-2 text-xs uppercase text-slate-400">
            <tr>
              <th class="px-4 py-2 text-left">Session id</th>
              <th class="px-4 py-2 text-left">URL</th>
              <th class="px-4 py-2 text-left">Title</th>
              <th class="px-4 py-2 text-right">Age</th>
              <th class="px-4 py-2 text-right">Idle</th>
              <th class="px-4 py-2 text-center"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-800">
            <tr
              v-for="s in snapshot.browser.sessions"
              :key="s.session_id"
              class="hover:bg-surface-2/60"
            >
              <td class="px-4 py-2 font-mono text-xs">{{ s.session_id }}</td>
              <td class="px-4 py-2 font-mono text-xs text-slate-400 truncate max-w-md">
                {{ s.url || 'about:blank' }}
              </td>
              <td class="px-4 py-2 text-xs truncate max-w-xs">
                {{ s.title || '—' }}
              </td>
              <td class="px-4 py-2 text-right font-mono text-xs text-slate-400">
                {{ fmtSecs(s.created_secs_ago) }}
              </td>
              <td class="px-4 py-2 text-right font-mono text-xs">
                <span :class="s.idle_secs > snapshot.browser.idle_timeout_secs * 0.75 ? 'text-amber-300' : 'text-slate-400'">
                  {{ fmtSecs(s.idle_secs) }}
                </span>
              </td>
              <td class="px-4 py-2 text-center">
                <button
                  type="button"
                  class="rounded bg-accent-err/20 px-2 py-0.5 text-xs text-accent-err hover:bg-accent-err/30"
                  @click="closeSession(s.session_id)"
                >
                  close
                </button>
              </td>
            </tr>
            <tr v-if="snapshot.browser.sessions.length === 0">
              <td colspan="6" class="px-4 py-6 text-center text-slate-500">
                No browser sessions open. The model opens one with
                <span class="font-mono">browser_open</span>.
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-if="closeError" class="mt-3 rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err">
        {{ closeError }}
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { inject, ref, type Ref } from 'vue'
import type { Snapshot } from '~/types/ws'
import { useRuntimeConfig } from '#app'

const feed = inject<{ snapshot: Ref<Snapshot | null> }>('dashboardFeed')!
const snapshot = feed.snapshot

const settingsOpen = ref(false)

const { patch: patchSettings } = useSettingsApi()
const patchError = ref<string | null>(null)
async function patch(body: Record<string, unknown>) {
  patchError.value = null
  try {
    await patchSettings('browser', body)
  } catch (e) {
    patchError.value = e instanceof Error ? e.message : String(e)
  }
}

const cfg = useRuntimeConfig()
const wsToken = (cfg.public.wsToken as string | undefined) ?? ''
const closeError = ref<string | null>(null)
const resetError = ref<string | null>(null)
async function resetPool(name: string) {
  resetError.value = null
  try {
    const headers: Record<string, string> = {}
    if (wsToken) headers.Authorization = `Bearer ${wsToken}`
    const res = await fetch(
      `/api/browser/pools/${encodeURIComponent(name)}/reset`,
      { method: 'POST', headers },
    )
    if (!res.ok) {
      const detail = await res.text().catch(() => '')
      throw new Error(`${res.status} ${res.statusText}: ${detail}`.trim())
    }
  } catch (e) {
    resetError.value = e instanceof Error ? e.message : String(e)
  }
}
async function closeSession(id: string) {
  closeError.value = null
  try {
    const headers: Record<string, string> = {}
    if (wsToken) headers.Authorization = `Bearer ${wsToken}`
    const res = await fetch(`/api/browser/sessions/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      headers,
    })
    if (!res.ok) {
      const detail = await res.text().catch(() => '')
      throw new Error(`${res.status} ${res.statusText}: ${detail}`.trim())
    }
  } catch (e) {
    closeError.value = e instanceof Error ? e.message : String(e)
  }
}

function fmtSecs(s: number): string {
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86400) return `${Math.floor(s / 3600)}h`
  return `${Math.floor(s / 86400)}d`
}
</script>
