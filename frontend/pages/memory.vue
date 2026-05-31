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
    <PageHeader title="Memory" @open-settings="settingsOpen = true" />

    <SettingsDrawer
      :open="settingsOpen"
      subsystem="Memory"
      @close="settingsOpen = false"
    >
      <div class="space-y-5 text-sm">
        <div>
          <label class="flex items-center justify-between gap-3">
            <span>
              <span class="font-medium text-slate-100">Show zero-value counts</span>
              <span class="block text-xs text-slate-400">
                Page-local UI toggle. When off, count cards that are 0 are
                hidden so the page reads as "what's actually populated."
              </span>
            </span>
            <input
              type="checkbox"
              class="h-5 w-5 accent-accent-info"
              v-model="showZeroCounts"
            />
          </label>
        </div>

        <hr class="border-slate-800" />

        <div class="space-y-3">
          <div class="text-xs uppercase tracking-wide text-slate-500">
            Restart required
          </div>
          <ReadOnlyRow label="Store directory" :value="snapshot.memory.db_path || '—'" />
          <ReadOnlyRow
            label="Embedding model"
            :value="snapshot.memory.embedding_model || 'token-based fallback'"
          />
        </div>
        <p class="text-xs text-slate-500">
          Memory retention, recall threshold, and the embedding endpoint live
          in <span class="font-mono">[memory]</span> and apply at startup.
        </p>
      </div>
    </SettingsDrawer>

    <section>
      <SectionHeading>Memos &amp; solutions</SectionHeading>
      <div class="grid grid-cols-2 gap-3 md:grid-cols-4">
        <StatCard
          v-if="show(snapshot.memory.memos)"
          label="Memos"
          :value="snapshot.memory.memos"
        />
        <StatCard
          v-if="show(snapshot.memory.solutions)"
          label="Solutions"
          :value="snapshot.memory.solutions"
        />
        <StatCard
          v-if="show(snapshot.memory.solution_revisions)"
          label="Solution revisions"
          :value="snapshot.memory.solution_revisions"
          :sub="`avg ${avgRevisions} per solution`"
        />
        <StatCard
          v-if="show(snapshot.memory.solution_tags)"
          label="Solution tags"
          :value="snapshot.memory.solution_tags"
        />
        <StatCard
          v-if="show(snapshot.memory.solution_links)"
          label="Solution links"
          :value="snapshot.memory.solution_links"
          sub="typed relation edges"
        />
        <StatCard
          v-if="show(snapshot.memory.solution_phrasings)"
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
          v-if="show(snapshot.memory.conversations)"
          label="Conversations"
          :value="snapshot.memory.conversations"
        />
        <StatCard
          v-if="show(snapshot.memory.conversation_turns)"
          label="Turns"
          :value="snapshot.memory.conversation_turns"
          :sub="`avg ${avgTurns} per conversation`"
        />
        <StatCard
          v-if="show(snapshot.memory.synonyms)"
          label="Synonyms"
          :value="snapshot.memory.synonyms"
          sub="learned token aliases"
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
const showZeroCounts = ref(true)

function show(n: number): boolean {
  return showZeroCounts.value || n > 0
}

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
