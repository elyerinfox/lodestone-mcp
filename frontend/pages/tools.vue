<!--
  Tools page — drill into the registered tool inventory. Groups by
  family prefix (`chart_*`, `solution_*`, `memory_*`, …) so the operator
  can see *which* tools are active vs gated off without calling
  tools/list over MCP. Free-text filter narrows both lists in real time.
-->
<template>
  <div v-if="!snapshot" class="text-slate-400">Waiting for snapshot…</div>
  <div v-else class="space-y-6">
    <PageHeader title="Tools" @open-settings="settingsOpen = true" />

    <SettingsDrawer
      :open="settingsOpen"
      subsystem="Tools"
      @close="settingsOpen = false"
    >
      <div class="space-y-5 text-sm">
        <label class="flex items-center justify-between gap-3">
          <span>
            <span class="font-medium text-slate-100">Compact rows</span>
            <span class="block text-xs text-slate-400">
              Tighter list spacing. Useful when scanning hundreds of tools.
            </span>
          </span>
          <input type="checkbox" class="h-5 w-5 accent-accent-info" v-model="compact" />
        </label>

        <label class="flex items-center justify-between gap-3">
          <span>
            <span class="font-medium text-slate-100">Hide families with no active tools</span>
            <span class="block text-xs text-slate-400">
              When on, families whose every tool is config-gated off
              are collapsed out of the list.
            </span>
          </span>
          <input
            type="checkbox"
            class="h-5 w-5 accent-accent-info"
            v-model="hideAllDisabledFamilies"
          />
        </label>

        <hr class="border-slate-800" />

        <div class="space-y-2">
          <div class="text-xs uppercase tracking-wide text-slate-500">
            Fully-disabled families
          </div>
          <p class="text-xs text-slate-400">
            Every tool in these families is hidden by config — usually
            because the whole skill is opted out.
          </p>
          <div v-if="fullyDisabledFamilies.length === 0" class="text-xs text-slate-500">
            None — every family has at least one active tool.
          </div>
          <ul v-else class="space-y-1">
            <li
              v-for="f in fullyDisabledFamilies"
              :key="f.family"
              class="flex items-center justify-between rounded border border-slate-800 bg-surface-2/40 px-3 py-1.5 text-xs"
            >
              <span class="font-mono text-slate-300">{{ f.family }}_*</span>
              <span class="text-slate-500">{{ f.count }} tool{{ f.count === 1 ? '' : 's' }}</span>
            </li>
          </ul>
        </div>

        <hr class="border-slate-800" />

        <div class="space-y-2">
          <div class="text-xs uppercase tracking-wide text-slate-500">
            Runtime-disabled
          </div>
          <p class="text-xs text-slate-400">
            Tools flipped off via the kill-switches on each row below.
            Disabled tools reject calls with an explanatory error and
            don't trigger auto-recall or conversation recording.
            Re-enable to restore.
          </p>
          <div
            v-if="snapshot.server.tools_runtime_disabled_names.length === 0"
            class="text-xs text-slate-500"
          >
            None — every active tool is reachable.
          </div>
          <ul v-else class="space-y-1">
            <li
              v-for="name in snapshot.server.tools_runtime_disabled_names"
              :key="`rd-${name}`"
              class="flex items-center justify-between rounded border border-amber-700/40 bg-amber-900/15 px-3 py-1.5 text-xs"
            >
              <span class="font-mono text-amber-200">{{ name }}</span>
              <button
                type="button"
                class="rounded bg-amber-700/30 px-2 py-0.5 text-xs text-amber-100 hover:bg-amber-700/50"
                @click="toggleTool(name, false)"
              >
                re-enable
              </button>
            </li>
          </ul>
          <div
            v-if="toolsPatchError"
            class="mt-2 rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err"
          >
            {{ toolsPatchError }}
          </div>
        </div>
      </div>
    </SettingsDrawer>

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
        <ul
          class="grid grid-cols-1 gap-x-4 text-xs md:grid-cols-2 lg:grid-cols-3"
          :class="compact ? 'px-4 py-2' : 'px-4 py-3'"
        >
          <li
            v-for="t in group.active"
            :key="`a-${t}`"
            class="group flex items-center gap-2 font-mono"
            :class="compact ? 'py-0' : 'py-0.5'"
          >
            <span
              class="h-1.5 w-1.5 rounded-full shrink-0"
              :class="
                runtimeDisabledSet.has(t) ? 'bg-amber-400' : 'bg-accent-ok'
              "
            />
            <span
              class="truncate"
              :class="
                runtimeDisabledSet.has(t)
                  ? 'text-amber-300 line-through opacity-80'
                  : 'text-slate-200'
              "
            >{{ t }}</span>
            <button
              type="button"
              class="ml-auto text-[10px] opacity-0 group-hover:opacity-100 focus:opacity-100"
              :class="
                runtimeDisabledSet.has(t)
                  ? 'text-accent-ok hover:underline'
                  : 'text-amber-400 hover:underline'
              "
              :title="runtimeDisabledSet.has(t) ? 'Re-enable at runtime' : 'Kill at runtime (no restart)'"
              @click="toggleTool(t, !runtimeDisabledSet.has(t))"
            >
              {{ runtimeDisabledSet.has(t) ? 'enable' : 'kill' }}
            </button>
          </li>
          <li
            v-for="t in group.disabled"
            :key="`d-${t}`"
            class="flex items-center gap-2 font-mono opacity-60"
            :class="compact ? 'py-0' : 'py-0.5'"
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
const settingsOpen = ref(false)
const compact = ref(false)
const hideAllDisabledFamilies = ref(false)

const runtimeDisabledSet = computed(
  () => new Set(snapshot.value?.server.tools_runtime_disabled_names ?? []),
)

const { patch: patchSettings } = useSettingsApi()
const toolsPatchError = ref<string | null>(null)
async function toggleTool(name: string, disabled: boolean) {
  toolsPatchError.value = null
  try {
    await patchSettings('tools', { disabled: { [name]: disabled } })
  } catch (e) {
    toolsPatchError.value = e instanceof Error ? e.message : String(e)
  }
}

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
    .filter((g) => !hideAllDisabledFamilies.value || g.active.length > 0)
})

// Families where every tool is config-gated off — surfaced in the
// settings drawer as the "skills you've opted out of" view.
const fullyDisabledFamilies = computed(() => {
  const all = buildGroups(
    snapshot.value?.server.tools_active_names ?? [],
    snapshot.value?.server.tools_disabled_names ?? [],
  )
  return all
    .filter((g) => g.active.length === 0 && g.disabled.length > 0)
    .map((g) => ({ family: g.family, count: g.disabled.length }))
})
</script>
