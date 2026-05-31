<!--
  Tools page — drill into the registered tool inventory. Groups by
  family prefix (`chart_*`, `solution_*`, `memory_*`, …) so the operator
  can see *which* tools are active vs gated off without calling
  tools/list over MCP. Free-text filter narrows both lists in real time.
-->
<template>
  <div v-if="!snapshot" class="text-slate-400">Waiting for snapshot…</div>
  <div v-else class="space-y-6">
    <section class="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
      <div>
        <SectionHeading>Tools</SectionHeading>
        <p class="text-sm text-slate-400">
          {{ activeTotal }} active · {{ disabledTotal }} hidden by config
          ·
          <span v-if="filter"
            >{{ filteredActiveCount + filteredDisabledCount }} match
            "<span class="font-mono">{{ filter }}</span>"</span
          >
          <span v-else>showing all</span>
        </p>
      </div>
      <div class="flex w-full items-center gap-2 md:w-80">
        <input
          v-model="filter"
          type="text"
          placeholder="filter by name (e.g. chart_, memory_get)"
          class="w-full rounded border border-slate-700 bg-surface-1 px-3 py-2 text-sm font-mono placeholder:text-slate-500 focus:border-accent-info focus:outline-none"
        />
        <button
          v-if="filter"
          class="rounded border border-slate-700 bg-surface-1 px-2 py-2 text-xs text-slate-400 hover:bg-surface-2"
          @click="filter = ''"
        >
          clear
        </button>
      </div>
    </section>

    <section class="flex gap-2 text-xs">
      <button
        class="rounded-full px-3 py-1"
        :class="
          showWhich === 'all'
            ? 'bg-accent-info/20 text-accent-info'
            : 'bg-surface-2 text-slate-400 hover:text-slate-200'
        "
        @click="showWhich = 'all'"
      >
        all ({{ activeTotal + disabledTotal }})
      </button>
      <button
        class="rounded-full px-3 py-1"
        :class="
          showWhich === 'active'
            ? 'bg-accent-ok/20 text-accent-ok'
            : 'bg-surface-2 text-slate-400 hover:text-slate-200'
        "
        @click="showWhich = 'active'"
      >
        active ({{ activeTotal }})
      </button>
      <button
        class="rounded-full px-3 py-1"
        :class="
          showWhich === 'disabled'
            ? 'bg-accent-warn/20 text-accent-warn'
            : 'bg-surface-2 text-slate-400 hover:text-slate-200'
        "
        @click="showWhich = 'disabled'"
      >
        disabled ({{ disabledTotal }})
      </button>
    </section>

    <section class="space-y-3">
      <div
        v-for="group in displayedGroups"
        :key="group.family"
        class="rounded-lg border border-slate-800 bg-surface-1"
      >
        <header
          class="flex items-center justify-between border-b border-slate-800 px-4 py-2"
        >
          <h3 class="font-mono text-sm font-semibold text-slate-200">
            {{ group.family }}_*
          </h3>
          <div class="text-xs text-slate-500">
            <span v-if="group.active.length > 0" class="text-accent-ok">
              {{ group.active.length }} active
            </span>
            <span
              v-if="group.active.length > 0 && group.disabled.length > 0"
              class="mx-1 text-slate-600"
              >·</span
            >
            <span v-if="group.disabled.length > 0" class="text-accent-warn">
              {{ group.disabled.length }} hidden
            </span>
          </div>
        </header>
        <ul class="grid grid-cols-1 gap-x-4 px-4 py-3 text-xs md:grid-cols-2 lg:grid-cols-3">
          <li
            v-for="t in group.active"
            :key="`a-${t}`"
            class="flex items-center gap-2 py-0.5 font-mono"
          >
            <span class="h-1.5 w-1.5 rounded-full bg-accent-ok shrink-0" />
            <span class="truncate text-slate-200">{{ t }}</span>
          </li>
          <li
            v-for="t in group.disabled"
            :key="`d-${t}`"
            class="flex items-center gap-2 py-0.5 font-mono opacity-60"
          >
            <span class="h-1.5 w-1.5 rounded-full bg-accent-warn shrink-0" />
            <span class="truncate text-slate-400 line-through">{{ t }}</span>
          </li>
        </ul>
      </div>
      <div
        v-if="displayedGroups.length === 0"
        class="rounded-lg border border-slate-800 bg-surface-1 px-4 py-8 text-center text-sm text-slate-500"
      >
        Nothing matches "<span class="font-mono">{{ filter }}</span>".
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, inject, ref, type Ref } from 'vue'
import type { Snapshot } from '~/types/ws'

const feed = inject<{ snapshot: Ref<Snapshot | null> }>('dashboardFeed')!
const snapshot = feed.snapshot

const filter = ref('')
const showWhich = ref<'all' | 'active' | 'disabled'>('all')

const activeTotal = computed(() => snapshot.value?.server.tools_active_names.length ?? 0)
const disabledTotal = computed(
  () => snapshot.value?.server.tools_disabled_names.length ?? 0,
)

// Family = the substring before the first underscore. `chart_line` →
// `chart`. Tools with no underscore land under "(misc)".
function family(name: string): string {
  const i = name.indexOf('_')
  return i < 0 ? '(misc)' : name.slice(0, i)
}

interface ToolGroup {
  family: string
  active: string[]
  disabled: string[]
}

function buildGroups(active: string[], disabled: string[]): ToolGroup[] {
  const map = new Map<string, ToolGroup>()
  for (const t of active) {
    const f = family(t)
    if (!map.has(f)) map.set(f, { family: f, active: [], disabled: [] })
    map.get(f)!.active.push(t)
  }
  for (const t of disabled) {
    const f = family(t)
    if (!map.has(f)) map.set(f, { family: f, active: [], disabled: [] })
    map.get(f)!.disabled.push(t)
  }
  // Stable family order: alphabetical, but stash "(misc)" at the end.
  return [...map.values()].sort((a, b) => {
    if (a.family === '(misc)') return 1
    if (b.family === '(misc)') return -1
    return a.family.localeCompare(b.family)
  })
}

const filteredActiveCount = computed(() => filtered.value.active.length)
const filteredDisabledCount = computed(() => filtered.value.disabled.length)

const filtered = computed(() => {
  const f = filter.value.trim().toLowerCase()
  const matches = (t: string) => f === '' || t.toLowerCase().includes(f)
  return {
    active: (snapshot.value?.server.tools_active_names ?? []).filter(matches),
    disabled: (snapshot.value?.server.tools_disabled_names ?? []).filter(matches),
  }
})

const displayedGroups = computed(() => {
  const all = buildGroups(filtered.value.active, filtered.value.disabled)
  return all
    .map((g) => ({
      family: g.family,
      active: showWhich.value === 'disabled' ? [] : g.active,
      disabled: showWhich.value === 'active' ? [] : g.disabled,
    }))
    .filter((g) => g.active.length + g.disabled.length > 0)
})
</script>
