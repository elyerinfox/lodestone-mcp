<!--
  Secret-presence row: label on the left, `<set>` or `<unset>` badge on
  the right. Mirrors the `<set>`/`<unset>` convention the `features`
  tool uses. The value is intentionally never accepted as a prop or
  displayed — only the boolean "is this configured" signal.
-->
<template>
  <div class="flex items-center justify-between rounded border border-slate-800 bg-surface-2/40 px-3 py-2">
    <span class="text-xs text-slate-400">{{ label }}</span>
    <span
      class="rounded px-2 py-0.5 text-xs font-mono"
      :class="set ? 'bg-accent-ok/20 text-accent-ok' : 'bg-surface-2 text-slate-500'"
    >{{ set ? '&lt;set&gt;' : '&lt;unset&gt;' }}</span>
  </div>
</template>

<script setup lang="ts">
defineProps<{ label: string; set: boolean }>()
</script>
