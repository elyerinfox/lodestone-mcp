<!--
  One labeled key/value row used inside SettingsDrawer panels for
  values that aren't runtime-tunable. The "restart required" framing
  comes from the surrounding section, so this component is just the
  presentation: label on the left, value on the right.
-->
<template>
  <div class="flex items-center justify-between rounded border border-slate-800 bg-surface-2/40 px-3 py-2">
    <span class="text-xs text-slate-400">{{ label }}</span>
    <span class="font-mono text-xs text-slate-200">{{ value }}</span>
  </div>
</template>

<script setup lang="ts">
defineProps<{ label: string; value: string | number }>()
</script>
