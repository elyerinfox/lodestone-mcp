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
        <label class="flex items-center justify-between gap-3">
          <span>
            <span class="font-medium text-slate-100">Memory family enabled</span>
            <span class="block text-xs text-slate-400">
              When off, the dispatch wrapper skips auto-recall and turn
              recording, and the memory_* / solution_* / conversation_*
              tools no-op. New rows aren't written.
            </span>
          </span>
          <input
            type="checkbox"
            class="h-5 w-5 accent-accent-info"
            :checked="snapshot.memory.enabled"
            @change="patchMemory({ enabled: ($event.target as HTMLInputElement).checked })"
          />
        </label>

        <label class="flex items-center justify-between gap-3">
          <span>
            <span class="font-medium text-slate-100">Auto-recall preamble</span>
            <span class="block text-xs text-slate-400">
              Prepends prior-solution recall to query-bearing tool responses.
              Turn off to silence the preamble without disabling memory.
            </span>
          </span>
          <input
            type="checkbox"
            class="h-5 w-5 accent-accent-info"
            :checked="snapshot.memory.auto_recall"
            :disabled="!snapshot.memory.enabled"
            @change="patchMemory({ auto_recall: ($event.target as HTMLInputElement).checked })"
          />
        </label>

        <label class="flex items-center justify-between gap-3">
          <span>
            <span class="font-medium text-slate-100">Record conversations</span>
            <span class="block text-xs text-slate-400">
              Writes one row per tool call to conversation_turns. Turn off
              to stop growing the conversation log without disabling memory.
            </span>
          </span>
          <input
            type="checkbox"
            class="h-5 w-5 accent-accent-info"
            :checked="snapshot.memory.record_conversations"
            :disabled="!snapshot.memory.enabled"
            @change="patchMemory({ record_conversations: ($event.target as HTMLInputElement).checked })"
          />
        </label>

        <div
          v-if="memPatchError"
          class="rounded border border-accent-err/40 bg-accent-err/10 p-2 text-xs text-accent-err"
        >
          {{ memPatchError }}
        </div>

        <hr class="border-slate-800" />

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
          Recall threshold, embedding endpoint, and retention live in
          <span class="font-mono">[memory]</span> and apply at startup.
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

    <section>
      <SectionHeading>Explorer</SectionHeading>
      <p class="mb-3 text-xs text-slate-400">
        Force-directed graph of recorded solutions and the typed links
        between them. Search to filter, click a node for the detail
        panel, double-click to re-root the view as a BFS focus subgraph.
      </p>
      <MemoryExplorer />
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

const { patch: patchSettings } = useSettingsApi()
const memPatchError = ref<string | null>(null)
async function patchMemory(body: Record<string, unknown>) {
  memPatchError.value = null
  try {
    await patchSettings('memory', body)
  } catch (e) {
    memPatchError.value = e instanceof Error ? e.message : String(e)
  }
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
