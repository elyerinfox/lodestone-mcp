<!--
  Generic stat card: big number + label + optional sub-line. Used by all
  three pages to keep the visual rhythm uniform.
-->
<template>
  <div class="rounded-lg border border-slate-800 bg-surface-1 px-5 py-4">
    <div class="text-xs uppercase tracking-wide text-slate-400">{{ label }}</div>
    <div class="mt-2 font-mono text-3xl text-slate-100">{{ value }}</div>
    <div v-if="sub" class="mt-1 text-xs text-slate-500">{{ sub }}</div>
  </div>
</template>

<script setup lang="ts">
defineProps<{
  label: string
  value: string | number
  sub?: string
}>()
</script>
