<!--
  Memory page — the full counts breakdown. No row bodies (the WebSocket
  feed never carries them per rule 11); this is the same SQL `COUNT(*)`
  surface the `features` tool already exposes, just rendered live.
-->
<template>
  <div v-if="!snapshot" class="text-slate-400">Waiting for snapshot…</div>
  <div v-else-if="!snapshot.memory.enabled" class="text-sm text-slate-500">
    Memory disabled — set
    <span class="font-mono">[memory].enabled = true</span>
    to populate this page.
  </div>
  <div v-else class="space-y-8">
    <section>
      <SectionHeading>Memos &amp; solutions</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard label="Memos" :value="snapshot.memory.memos" />
        <StatCard label="Solutions" :value="snapshot.memory.solutions" />
        <StatCard
          label="Solution revisions"
          :value="snapshot.memory.solution_revisions"
          :sub="`avg ${avgRevisions} per solution`"
        />
        <StatCard
          label="Solution tags"
          :value="snapshot.memory.solution_tags"
        />
        <StatCard
          label="Solution links"
          :value="snapshot.memory.solution_links"
          sub="typed relation edges"
        />
        <StatCard
          label="Solution phrasings"
          :value="snapshot.memory.solution_phrasings"
          sub="alternate-question recall aliases"
        />
      </div>
    </section>

    <section>
      <SectionHeading>Conversations</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-3">
        <StatCard
          label="Conversations"
          :value="snapshot.memory.conversations"
        />
        <StatCard
          label="Turns"
          :value="snapshot.memory.conversation_turns"
          :sub="`avg ${avgTurns} per conversation`"
        />
        <StatCard
          label="Synonyms"
          :value="snapshot.memory.synonyms"
          sub="learned token aliases"
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

const avgRevisions = computed(() => {
  const m = snapshot.value?.memory
  if (!m || m.solutions === 0) return '—'
  return (m.solution_revisions / m.solutions).toFixed(1)
})
const avgTurns = computed(() => {
  const m = snapshot.value?.memory
  if (!m || m.conversations === 0) return '—'
  return (m.conversation_turns / m.conversations).toFixed(1)
})
</script>
