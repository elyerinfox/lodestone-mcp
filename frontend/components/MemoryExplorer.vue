<!--
  Inline solution-graph explorer for the Memory page. Search is the
  primary entry point: typing immediately filters the visible graph
  (debounced 250 ms, server-side via `mode=filter&query=`). A tag
  chip + hide-superseded toggle live alongside it. Click any node to
  open the detail panel; double-click (or hit "focus on this") to
  re-root the view via `mode=focus` with BFS to `depth` hops.

  Layout is d3-force with a hand-rolled SVG render so we stay
  consistent with the existing constellation graph aesthetic.
-->
<template>
  <div class="space-y-4">
    <!-- Search + filters bar -->
    <div class="flex flex-wrap items-center gap-2">
      <input
        v-model="searchInput"
        type="search"
        :placeholder="placeholder"
        class="flex-1 min-w-[200px] rounded border border-slate-700 bg-surface-0 px-3 py-2 text-sm placeholder:text-slate-500 focus:border-accent-info focus:outline-none"
      />
      <select
        v-model="tagInput"
        class="rounded border border-slate-700 bg-surface-0 px-2 py-2 text-xs"
      >
        <option value="">all tags</option>
        <option v-for="t in knownTags" :key="t" :value="t">{{ t }}</option>
      </select>
      <label class="flex items-center gap-1.5 text-xs text-slate-300">
        <input type="checkbox" v-model="hideSuperseded" class="accent-accent-info" />
        hide superseded
      </label>
      <button
        v-if="focusId"
        type="button"
        class="rounded border border-slate-700 bg-surface-1 px-3 py-2 text-xs hover:bg-surface-2"
        @click="exitFocus"
      >
        exit focus
      </button>
      <span class="ml-auto text-xs text-slate-500">
        {{ graph.nodes.length }} node{{ graph.nodes.length === 1 ? '' : 's' }},
        {{ graph.edges.length }} edge{{ graph.edges.length === 1 ? '' : 's' }}
      </span>
    </div>

    <!-- Focus mode controls (only when a focus node is set) -->
    <div v-if="focusId" class="flex flex-wrap items-center gap-3 text-xs">
      <span class="text-slate-400">
        focus:
        <span class="font-mono text-amber-300">{{ focusId }}</span>
      </span>
      <label class="flex items-center gap-1.5 text-slate-300">
        depth
        <input
          type="number"
          v-model.number="focusDepth"
          min="1"
          max="5"
          class="w-12 rounded border border-slate-700 bg-surface-0 px-1.5 py-0.5"
        />
      </label>
    </div>

    <div
      v-if="error"
      class="rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err"
    >
      {{ error }}
    </div>

    <!-- Graph canvas -->
    <div
      class="relative overflow-hidden rounded-lg border border-slate-800 bg-surface-0"
      :style="{ height: '520px' }"
    >
      <svg
        :viewBox="`0 0 ${width} ${height}`"
        class="block h-full w-full"
        preserveAspectRatio="xMidYMid meet"
      >
        <g>
          <line
            v-for="(e, i) in edgesView"
            :key="`e-${i}`"
            :x1="e.x1"
            :y1="e.y1"
            :x2="e.x2"
            :y2="e.y2"
            :stroke="edgeStroke(e.kind)"
            stroke-width="1.4"
            stroke-linecap="round"
          />
          <g
            v-for="n in nodesView"
            :key="n.id"
            :transform="`translate(${n.x},${n.y})`"
            @click="select(n.id)"
            @dblclick="recenter(n.id)"
            class="cursor-pointer"
          >
            <circle
              :r="nodeRadius(n)"
              :fill="nodeFill(n)"
              :stroke="selected === n.id ? '#f59e0b' : '#0f1115'"
              :stroke-width="selected === n.id ? 2.4 : 1.5"
            />
            <text
              y="3"
              text-anchor="middle"
              fill="#0f1115"
              font-size="8"
              font-weight="600"
              font-family="ui-monospace,Menlo,Consolas,monospace"
            >
              {{ n.id }}
            </text>
          </g>
        </g>
      </svg>
      <div
        v-if="loading"
        class="pointer-events-none absolute inset-0 flex items-center justify-center bg-surface-0/40 text-xs text-slate-300"
      >
        loading…
      </div>
      <div
        v-else-if="graph.nodes.length === 0"
        class="pointer-events-none absolute inset-0 flex items-center justify-center p-6 text-center text-sm text-slate-500"
      >
        <span v-if="searchInput || tagInput || hideSuperseded || focusId">
          Nothing matches the current filter.
        </span>
        <span v-else>
          No solutions recorded yet — use the
          <span class="mx-1 font-mono">remember_solution</span> tool to seed one.
        </span>
      </div>
    </div>

    <!-- Detail panel -->
    <div
      v-if="selectedNode"
      class="rounded-lg border border-slate-800 bg-surface-1 p-4"
    >
      <header class="mb-3 flex items-start justify-between gap-4">
        <div>
          <div class="text-xs uppercase tracking-wide text-slate-500">
            {{ selectedNode.id }} · revision {{ selectedNode.revision_count }}
          </div>
          <h3 class="mt-1 text-sm font-medium text-slate-100">
            {{ selectedNode.problem }}
          </h3>
        </div>
        <button
          type="button"
          class="rounded p-1 text-slate-400 hover:bg-surface-2 hover:text-slate-100"
          @click="selected = null"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M6 6l12 12M6 18L18 6" />
          </svg>
        </button>
      </header>

      <p v-if="selectedNode.summary" class="mb-3 text-xs text-slate-300">
        {{ selectedNode.summary }}
      </p>

      <div
        v-if="selectedNode.superseded_by_head && selectedNode.superseded_by_head !== selectedNode.id"
        class="mb-3 rounded border border-amber-700/40 bg-amber-900/15 p-2 text-xs text-amber-200"
      >
        ⚠ Superseded — current head is
        <button
          type="button"
          class="font-mono underline hover:text-amber-100"
          @click="select(selectedNode.superseded_by_head!)"
        >{{ selectedNode.superseded_by_head }}</button>.
        Prefer it unless you specifically need this older revision.
      </div>

      <div v-if="selectedNode.tags.length > 0" class="mb-3 flex flex-wrap gap-1">
        <button
          v-for="t in selectedNode.tags"
          :key="t"
          type="button"
          class="rounded bg-surface-2 px-2 py-0.5 text-xs font-mono text-slate-300 hover:bg-surface-2/80"
          @click="tagInput = t"
        >{{ t }}</button>
      </div>

      <div v-if="selectedEdges.length > 0" class="space-y-1 text-xs">
        <div class="text-slate-500 uppercase tracking-wide">Links</div>
        <ul class="space-y-1">
          <li v-for="(e, i) in selectedEdges" :key="i" class="font-mono">
            <span class="text-slate-400">─{{ e.kind }}→</span>
            <button
              type="button"
              class="ml-2 text-accent-info hover:underline"
              @click="select(e.other)"
            >{{ e.other }}</button>
          </li>
        </ul>
      </div>

      <div class="mt-3 text-xs text-slate-500">
        <button
          type="button"
          class="font-mono hover:text-accent-info"
          @click="recenter(selectedNode.id)"
        >focus on this node (double-click works too)</button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  forceCenter,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceCollide,
  type Simulation,
} from 'd3-force'
import type { MemoryGraph } from '~/types/ws'

const { fetchGraph } = useMemoryGraph()

// State ----------------------------------------------------------------------
const graph = ref<MemoryGraph>({ nodes: [], edges: [] })
const loading = ref(false)
const error = ref<string | null>(null)
const selected = ref<string | null>(null)

const searchInput = ref('')
const tagInput = ref('')
const hideSuperseded = ref(false)
const focusId = ref<string | null>(null)
const focusDepth = ref(2)

const width = 1000
const height = 520

const placeholder = computed(() => {
  if (focusId.value) return `focused on ${focusId.value}`
  return 'Search problem or summary…'
})

// Distinct tag list for the dropdown — refreshed from the loaded data.
const knownTags = computed(() => {
  const set = new Set<string>()
  for (const n of graph.value.nodes) {
    for (const t of n.tags) set.add(t)
  }
  return Array.from(set).sort()
})

// Force-directed sim ---------------------------------------------------------
interface SimNode {
  id: string
  x: number
  y: number
  vx?: number
  vy?: number
}
interface SimLink {
  source: SimNode | string
  target: SimNode | string
  kind: string
}

const simNodes = ref<SimNode[]>([])
const simLinks = ref<SimLink[]>([])
let sim: Simulation<SimNode, SimLink> | null = null
const tickToken = ref(0)

function stopSim() {
  if (sim) {
    sim.stop()
    sim = null
  }
}

function rebuildSim() {
  stopSim()
  const prev = new Map(simNodes.value.map((n) => [n.id, n]))
  simNodes.value = graph.value.nodes.map((n) => {
    const old = prev.get(n.id)
    return {
      id: n.id,
      x: old?.x ?? width / 2 + (Math.random() - 0.5) * 200,
      y: old?.y ?? height / 2 + (Math.random() - 0.5) * 200,
    }
  })
  simLinks.value = graph.value.edges.map((e) => ({
    source: e.from,
    target: e.to,
    kind: e.kind,
  }))
  sim = forceSimulation<SimNode, SimLink>(simNodes.value)
    .force(
      'link',
      forceLink<SimNode, SimLink>(simLinks.value)
        .id((d) => d.id)
        .distance(80)
        .strength(0.5),
    )
    .force('charge', forceManyBody().strength(-200))
    .force('collide', forceCollide<SimNode>().radius(22))
    .force('center', forceCenter(width / 2, height / 2))
    .alpha(1)
    .on('tick', () => {
      tickToken.value++
    })
}

const nodesView = computed(() => {
  void tickToken.value
  const meta = new Map(graph.value.nodes.map((n) => [n.id, n]))
  return simNodes.value.map((n) => ({
    id: n.id,
    x: n.x,
    y: n.y,
    meta: meta.get(n.id),
  }))
})

const edgesView = computed(() => {
  void tickToken.value
  const byId = new Map(simNodes.value.map((n) => [n.id, n]))
  return simLinks.value.flatMap((l) => {
    const a =
      typeof l.source === 'string' ? byId.get(l.source) : (l.source as SimNode)
    const b =
      typeof l.target === 'string' ? byId.get(l.target) : (l.target as SimNode)
    if (!a || !b) return []
    return [{ x1: a.x, y1: a.y, x2: b.x, y2: b.y, kind: l.kind }]
  })
})

function nodeRadius(n: { meta?: { revision_count?: number } }) {
  const r = n.meta?.revision_count ?? 1
  return 6 + Math.min(r, 5) * 1.2
}
function nodeFill(n: { id: string; meta?: { superseded_by_head?: string | null } }) {
  if (selected.value === n.id) return '#f59e0b'
  if (focusId.value === n.id) return '#fbbf24'
  if (n.meta?.superseded_by_head) return '#64748b'
  return '#60a5fa'
}
function edgeStroke(kind: string): string {
  switch (kind) {
    case 'supersedes':
    case 'superseded-by':
      return '#f59e0b66'
    case 'depends-on':
    case 'dependency-of':
      return '#34d39966'
    default:
      return '#475569aa'
  }
}

// Selection ------------------------------------------------------------------
const selectedNode = computed(() => {
  if (!selected.value) return null
  return graph.value.nodes.find((n) => n.id === selected.value) ?? null
})
const selectedEdges = computed(() => {
  if (!selected.value) return []
  return graph.value.edges
    .filter((e) => e.from === selected.value || e.to === selected.value)
    .map((e) => ({
      kind: e.kind,
      other: e.from === selected.value ? e.to : e.from,
    }))
})

function select(id: string) {
  selected.value = id
}
function recenter(id: string) {
  focusId.value = id
  loadGraph()
}
function exitFocus() {
  focusId.value = null
  loadGraph()
}

// Debounced load -------------------------------------------------------------
let debounceTimer: ReturnType<typeof setTimeout> | null = null
function scheduleLoad() {
  if (debounceTimer) clearTimeout(debounceTimer)
  debounceTimer = setTimeout(loadGraph, 250)
}

async function loadGraph() {
  loading.value = true
  error.value = null
  try {
    if (focusId.value) {
      graph.value = await fetchGraph('focus', {
        id: focusId.value,
        depth: focusDepth.value,
      })
    } else if (searchInput.value || tagInput.value || hideSuperseded.value) {
      graph.value = await fetchGraph('filter', {
        tag: tagInput.value || undefined,
        query: searchInput.value || undefined,
        hide_superseded: hideSuperseded.value || undefined,
      })
    } else {
      graph.value = await fetchGraph('all')
    }
    rebuildSim()
    // If the previously-selected node is no longer in the result set,
    // drop the selection so the detail panel doesn't render stale state.
    if (
      selected.value &&
      !graph.value.nodes.some((n) => n.id === selected.value)
    ) {
      selected.value = null
    }
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

watch([searchInput, tagInput, hideSuperseded], () => {
  if (focusId.value) return // focus mode ignores the filter inputs
  scheduleLoad()
})
watch(focusDepth, () => {
  if (focusId.value) scheduleLoad()
})

onMounted(loadGraph)
onBeforeUnmount(stopSim)
</script>
