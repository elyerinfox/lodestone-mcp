<!--
  Radial topology graph — this node at the center, peers as labeled
  spokes around a circle. Pure SVG, no chart library, no extra
  dependencies. Visual encoding mirrors the table below:

    line style    : solid green  = reachable (digest seen recently)
                    dashed amber = unreachable (still being retried)
    edge thickness: peer's reputation (0.5..1.0 → 1px..3px)
    node fill     : green   = reachable AND delegation_enabled
                    blue    = reachable, delegation off
                    grey    = unreachable
    node ring     : a faint outer ring when delegation_enabled, so the
                    "willing to serve us a fetch" peers stand out at a
                    glance.

  The legend at the bottom calls these out so the operator doesn't have
  to guess.
-->
<template>
  <div class="rounded-lg border border-slate-800 bg-surface-1 p-5">
    <svg
      :viewBox="`0 0 ${size} ${size}`"
      class="block w-full max-w-2xl"
      preserveAspectRatio="xMidYMid meet"
    >
      <!-- The orbit ring — a visual anchor for the radial layout. -->
      <circle
        :cx="center"
        :cy="center"
        :r="orbitRadius"
        fill="none"
        stroke="#1d2230"
        stroke-width="1"
      />
      <!-- Edges from center → each peer. -->
      <g>
        <line
          v-for="(p, i) in placedPeers"
          :key="`e-${p.url}`"
          :x1="center"
          :y1="center"
          :x2="p.x"
          :y2="p.y"
          :stroke="p.reachable ? '#34d39955' : '#fbbf2455'"
          :stroke-width="1 + Math.max(0, p.peer.reputation - 0.5) * 4"
          :stroke-dasharray="p.reachable ? '0' : '4 4'"
        />
        <!-- Peer node + label. -->
        <g v-for="p in placedPeers" :key="`n-${p.url}`">
          <circle
            v-if="p.peer.delegation_enabled"
            :cx="p.x"
            :cy="p.y"
            :r="peerRadius + 4"
            fill="none"
            stroke="#60a5fa55"
            stroke-width="1.5"
          />
          <circle
            :cx="p.x"
            :cy="p.y"
            :r="peerRadius"
            :fill="nodeFill(p)"
            stroke="#0f1115"
            stroke-width="1.5"
          />
          <text
            :x="p.labelX"
            :y="p.labelY"
            :text-anchor="p.anchor"
            dominant-baseline="middle"
            fill="#cbd5e1"
            font-size="11"
            font-family="ui-monospace,Menlo,Consolas,monospace"
          >
            {{ p.peer.node_id ?? hostFromUrl(p.peer.url) }}
          </text>
        </g>
      </g>
      <!-- The center "me" node. -->
      <circle
        :cx="center"
        :cy="center"
        :r="centerRadius"
        fill="#60a5fa"
        stroke="#0f1115"
        stroke-width="2"
      />
      <text
        :x="center"
        :y="center"
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
        :x="center"
        :y="center + centerRadius + 14"
        text-anchor="middle"
        fill="#cbd5e1"
        font-size="10"
        font-family="ui-monospace,Menlo,Consolas,monospace"
      >
        {{ nodeId }}
      </text>
      <!-- Empty-state copy lives inside the SVG so it scales with the box. -->
      <text
        v-if="placedPeers.length === 0"
        :x="center"
        :y="size - 20"
        text-anchor="middle"
        fill="#64748b"
        font-size="11"
      >
        No peers known yet — mDNS may still be discovering.
      </text>
    </svg>
    <!-- Legend -->
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
          class="inline-block h-3 w-3 rounded-full border-2 border-accent-info bg-transparent"
        />
        delegation enabled
      </span>
      <span class="flex items-center gap-2 font-mono">
        — line: solid = reachable · dashed = unreachable · width ∝ reputation
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
}>()

// Generous viewBox so labels never clip even when many peers are placed.
const size = 480
const center = size / 2
const orbitRadius = 170
const peerRadius = 8
const centerRadius = 18
const labelGap = 14 // distance from peer node edge to label baseline

interface PlacedPeer {
  url: string
  peer: PeerEntry
  reachable: boolean
  x: number
  y: number
  labelX: number
  labelY: number
  anchor: 'start' | 'end' | 'middle'
}

// Distribute peers evenly around the circle. Sorted by URL so the
// layout is stable across snapshot refreshes (no thrashing).
const placedPeers = computed<PlacedPeer[]>(() => {
  const peers = [...props.peers].sort((a, b) => a.url.localeCompare(b.url))
  const n = peers.length
  if (n === 0) return []
  return peers.map((peer, i) => {
    // Start at -π/2 so the first peer sits at the top of the circle.
    const angle = -Math.PI / 2 + (i * 2 * Math.PI) / n
    const x = center + orbitRadius * Math.cos(angle)
    const y = center + orbitRadius * Math.sin(angle)
    // Label outside the node, away from center.
    const labelX = center + (orbitRadius + peerRadius + labelGap) * Math.cos(angle)
    const labelY = center + (orbitRadius + peerRadius + labelGap) * Math.sin(angle)
    // Anchor based on which half of the circle the peer sits on, so the
    // label reads outward.
    let anchor: 'start' | 'end' | 'middle'
    const cos = Math.cos(angle)
    if (Math.abs(cos) < 0.2) anchor = 'middle'
    else if (cos > 0) anchor = 'start'
    else anchor = 'end'
    return {
      url: peer.url,
      peer,
      reachable: peer.reachable,
      x,
      y,
      labelX,
      labelY,
      anchor,
    }
  })
})

function nodeFill(p: PlacedPeer): string {
  if (!p.reachable) return '#64748b'
  if (p.peer.delegation_enabled) return '#34d399'
  return '#60a5fa'
}

function hostFromUrl(url: string): string {
  try {
    const u = new URL(url)
    return u.host
  } catch {
    return url
  }
}
</script>
