<!--
  Swarm topology — every node we know about (us + direct peers + peers
  mentioned only by other peers) on a circle, with edges for every
  peer-to-peer adjacency our gossip data has surfaced. Pure SVG. Visual
  encoding:

    node fill     : green   = reachable AND delegation_enabled
                    blue    = reachable, delegation off
                    grey    = unreachable (we know about it but can't
                              currently reach its digest)
                    outline = indirect (only mentioned by another peer
                              — we don't talk to it directly)
                    accent  = us
    node ring     : faint outer ring when the peer advertised
                    delegation_enabled.
    edge style    : solid     = reciprocal (both endpoints list each
                                other in their known-peers)
                    dashed    = one-way (only one endpoint sees the
                                other — partition or just-discovered)
    edge thickness: edges touching a direct peer scale with that peer's
                    reputation (0.5..1.0 → 1px..3px); indirect↔indirect
                    edges are a flat 1px.

  Layout is a deterministic ring keyed by URL so refreshes don't reshuffle
  positions.
-->
<template>
  <div class="rounded-lg border border-slate-800 bg-surface-1 p-5">
    <svg
      :viewBox="`0 0 ${size} ${size}`"
      class="block w-full max-w-2xl"
      preserveAspectRatio="xMidYMid meet"
    >
      <circle
        :cx="center"
        :cy="center"
        :r="orbitRadius"
        fill="none"
        stroke="#1d2230"
        stroke-width="1"
      />
      <!-- Edges first so nodes draw on top of them. -->
      <g>
        <line
          v-for="e in edges"
          :key="e.key"
          :x1="e.x1"
          :y1="e.y1"
          :x2="e.x2"
          :y2="e.y2"
          :stroke="edgeStroke(e)"
          :stroke-width="e.width"
          :stroke-dasharray="e.reciprocal ? '0' : '4 4'"
          stroke-linecap="round"
        />
      </g>
      <g>
        <g v-for="n in placedNodes" :key="`n-${n.id}`">
          <!-- Delegation halo for direct peers that advertised it. -->
          <circle
            v-if="n.delegationEnabled"
            :cx="n.x"
            :cy="n.y"
            :r="nodeRadius(n) + 4"
            fill="none"
            stroke="#60a5fa55"
            stroke-width="1.5"
          />
          <circle
            :cx="n.x"
            :cy="n.y"
            :r="nodeRadius(n)"
            :fill="nodeFill(n)"
            :stroke="nodeStroke(n)"
            :stroke-width="n.indirect ? 1.5 : 2"
            :stroke-dasharray="n.indirect ? '3 2' : '0'"
          />
          <text
            v-if="n.self"
            :x="n.x"
            :y="n.y"
            text-anchor="middle"
            dominant-baseline="middle"
            fill="#0f1115"
            font-size="11"
            font-weight="600"
            font-family="ui-monospace,Menlo,Consolas,monospace"
          >
            me
          </text>
          <text
            :x="n.labelX"
            :y="n.labelY"
            :text-anchor="n.anchor"
            dominant-baseline="middle"
            fill="#cbd5e1"
            font-size="11"
            font-family="ui-monospace,Menlo,Consolas,monospace"
          >
            {{ n.label }}
          </text>
        </g>
      </g>
      <text
        v-if="placedNodes.length <= 1"
        :x="center"
        :y="size - 20"
        text-anchor="middle"
        fill="#64748b"
        font-size="11"
      >
        No peers known yet — mDNS may still be discovering.
      </text>
    </svg>
    <div class="mt-3 flex flex-wrap items-center gap-x-5 gap-y-2 text-xs text-slate-400">
      <span class="flex items-center gap-2">
        <span class="h-2.5 w-2.5 rounded-full bg-accent-ok" />
        reachable
      </span>
      <span class="flex items-center gap-2">
        <span class="h-2.5 w-2.5 rounded-full bg-accent-info" />
        reachable (delegation off)
      </span>
      <span class="flex items-center gap-2">
        <span class="h-2.5 w-2.5 rounded-full bg-slate-500" />
        unreachable
      </span>
      <span class="flex items-center gap-2">
        <span
          class="inline-block h-3 w-3 rounded-full border border-dashed border-slate-400 bg-transparent"
        />
        indirect (peer-of-peer)
      </span>
      <span class="flex items-center gap-2 font-mono">
        — edge: solid = reciprocal · dashed = one-way
      </span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { PeerEntry } from '~/types/ws'

const props = defineProps<{
  nodeId: string
  peers: PeerEntry[]
  localUrls?: string[]
}>()

const size = 520
const center = size / 2
const orbitRadius = 190
const directRadius = 9
const indirectRadius = 6
const meRadius = 18
const labelGap = 14

interface GraphNode {
  id: string // normalized URL or 'self'
  url: string
  label: string
  self: boolean
  indirect: boolean // true when we only learned about it through another peer
  reachable: boolean
  delegationEnabled: boolean
  reputation: number
}

interface PlacedNode extends GraphNode {
  x: number
  y: number
  labelX: number
  labelY: number
  anchor: 'start' | 'end' | 'middle'
}

interface EdgeInput {
  from: string
  to: string
  reputation: number // best reputation seen on either endpoint when known
  reciprocal: boolean
}

interface Edge {
  key: string
  x1: number
  y1: number
  x2: number
  y2: number
  width: number
  reciprocal: boolean
  touchesDirect: boolean
}

function normalize(url: string): string {
  // Mirror src/constellation/mod.rs::normalize_base: lowercase, strip
  // trailing slashes. Frontend doesn't need to be perfect — close enough
  // that two peers referring to the same address match.
  return url.trim().toLowerCase().replace(/\/+$/, '')
}

function hostFromUrl(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}

// Build the node set + adjacency map from the snapshot.
const graph = computed(() => {
  const selfId = 'self'
  const localUrls = new Set((props.localUrls ?? []).map(normalize))

  const nodes = new Map<string, GraphNode>()
  nodes.set(selfId, {
    id: selfId,
    url: '(this node)',
    label: props.nodeId.slice(0, 12),
    self: true,
    indirect: false,
    reachable: true,
    delegationEnabled: false,
    reputation: 1,
  })

  // Map normalized URL → node id, so peer-known-URLs can resolve back
  // to the self node even when a peer refers to us by a LAN address.
  const urlToId = new Map<string, string>()
  for (const u of localUrls) urlToId.set(u, selfId)

  // First pass: direct peers become nodes. When two peer URLs share a
  // (known) node_id — mDNS routinely resolves one machine at every
  // interface address — collapse them to one graph node. The reachable
  // URL wins for reach/delegation flags; the other URLs are aliases
  // that route to the same node for peer-of-peer edge matching.
  for (const p of props.peers) {
    const norm = normalize(p.url)
    const id = p.node_id ?? norm
    urlToId.set(norm, id)
    const existing = nodes.get(id)
    if (!existing) {
      nodes.set(id, {
        id,
        url: p.url,
        label: p.node_id ? p.node_id.slice(0, 12) : hostFromUrl(p.url),
        self: false,
        indirect: false,
        reachable: p.reachable,
        delegationEnabled: p.delegation_enabled,
        reputation: p.reputation,
      })
    } else {
      // Prefer the reachable entry's encoding.
      if (p.reachable && !existing.reachable) {
        existing.url = p.url
        existing.reachable = true
        existing.delegationEnabled = p.delegation_enabled
        existing.reputation = p.reputation
      } else {
        existing.reputation = Math.max(existing.reputation, p.reputation)
      }
    }
  }

  // Second pass: peer-of-peer URLs that aren't direct peers become
  // "indirect" nodes.
  for (const p of props.peers) {
    for (const k of p.known_peers ?? []) {
      const norm = normalize(k)
      if (urlToId.has(norm)) continue
      urlToId.set(norm, norm)
      nodes.set(norm, {
        id: norm,
        url: k,
        label: hostFromUrl(k),
        self: false,
        indirect: true,
        reachable: false,
        delegationEnabled: false,
        reputation: 0.5,
      })
    }
  }

  // Build edges: directed first, then collapse to undirected with a
  // reciprocal flag.
  const directed = new Map<string, { reputation: number }>()
  const directedKey = (a: string, b: string) => `${a}|${b}`

  // We → every direct peer (gossip pull = we contact them).
  for (const p of props.peers) {
    const id = urlToId.get(normalize(p.url))
    if (!id) continue
    directed.set(directedKey(selfId, id), { reputation: p.reputation })
  }
  // Each peer → its known_peers.
  for (const p of props.peers) {
    const from = urlToId.get(normalize(p.url))
    if (!from) continue
    for (const k of p.known_peers ?? []) {
      const to = urlToId.get(normalize(k))
      if (!to || to === from) continue
      directed.set(directedKey(from, to), { reputation: p.reputation })
    }
  }

  const undirected = new Map<string, EdgeInput>()
  for (const [k, v] of directed) {
    const [a, b] = k.split('|')
    const lo = a < b ? a : b
    const hi = a < b ? b : a
    const key = `${lo}|${hi}`
    const reverse = directedKey(b, a)
    const reciprocal = directed.has(reverse)
    const existing = undirected.get(key)
    const rep = existing
      ? Math.max(existing.reputation, v.reputation)
      : v.reputation
    undirected.set(key, { from: lo, to: hi, reputation: rep, reciprocal })
  }

  return { nodes: Array.from(nodes.values()), edges: Array.from(undirected.values()) }
})

// Place nodes on a ring. Self stays at -π/2 (top); peers fill the rest
// in URL order so positions are stable across refreshes.
const placedNodes = computed<PlacedNode[]>(() => {
  const { nodes } = graph.value
  if (nodes.length === 0) return []
  const others = nodes
    .filter((n) => !n.self)
    .sort((a, b) => a.id.localeCompare(b.id))
  const ordered = [nodes.find((n) => n.self)!, ...others]
  const n = ordered.length
  return ordered.map((node, i) => {
    const angle = -Math.PI / 2 + (i * 2 * Math.PI) / n
    const x = center + orbitRadius * Math.cos(angle)
    const y = center + orbitRadius * Math.sin(angle)
    const r = node.self ? meRadius : node.indirect ? indirectRadius : directRadius
    const labelX = center + (orbitRadius + r + labelGap) * Math.cos(angle)
    const labelY = center + (orbitRadius + r + labelGap) * Math.sin(angle)
    let anchor: 'start' | 'end' | 'middle'
    const cos = Math.cos(angle)
    if (Math.abs(cos) < 0.2) anchor = 'middle'
    else if (cos > 0) anchor = 'start'
    else anchor = 'end'
    return { ...node, x, y, labelX, labelY, anchor }
  })
})

const nodeById = computed(() => {
  const m = new Map<string, PlacedNode>()
  for (const n of placedNodes.value) m.set(n.id, n)
  return m
})

const edges = computed<Edge[]>(() => {
  const out: Edge[] = []
  for (const e of graph.value.edges) {
    const a = nodeById.value.get(e.from)
    const b = nodeById.value.get(e.to)
    if (!a || !b) continue
    const touchesDirect = !a.indirect || !b.indirect
    const width = touchesDirect ? 1 + Math.max(0, e.reputation - 0.5) * 4 : 1
    out.push({
      key: `${e.from}|${e.to}`,
      x1: a.x,
      y1: a.y,
      x2: b.x,
      y2: b.y,
      width,
      reciprocal: e.reciprocal,
      touchesDirect,
    })
  }
  return out
})

function nodeRadius(n: PlacedNode): number {
  if (n.self) return meRadius
  return n.indirect ? indirectRadius : directRadius
}

function nodeFill(n: PlacedNode): string {
  if (n.self) return '#60a5fa'
  if (n.indirect) return 'transparent'
  if (!n.reachable) return '#64748b'
  if (n.delegationEnabled) return '#34d399'
  return '#60a5fa'
}

function nodeStroke(n: PlacedNode): string {
  if (n.self) return '#0f1115'
  if (n.indirect) return '#94a3b8'
  return '#0f1115'
}

function edgeStroke(e: Edge): string {
  if (!e.touchesDirect) return '#33415544'
  return e.reciprocal ? '#34d39966' : '#fbbf2455'
}
</script>
