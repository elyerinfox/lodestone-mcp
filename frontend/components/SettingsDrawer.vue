<!--
  Reusable slide-out settings drawer. The PAGE owns the actual controls
  via the default slot; this shell just handles open/close, the
  "ephemeral" disclosure banner, and the page-level title. The
  ephemeral banner is non-negotiable: every settings panel applies to
  the running process only, not to disk, and the operator needs to know
  that at a glance so they don't expect changes to survive a restart.

  Open state is page-local: each page that uses the drawer keeps its
  own ref, and the gear button next to the page heading toggles it.
-->
<template>
  <Teleport to="body">
    <transition name="fade">
      <div
        v-if="open"
        class="fixed inset-0 z-40 bg-black/40"
        @click="$emit('close')"
      />
    </transition>
    <transition name="slide">
      <aside
        v-if="open"
        class="fixed right-0 top-0 z-50 h-full w-full max-w-md overflow-y-auto border-l border-slate-800 bg-surface-1 p-6 shadow-2xl"
        role="dialog"
        aria-modal="true"
        :aria-label="`${subsystem} settings`"
      >
        <header class="mb-4 flex items-center justify-between">
          <div>
            <h2 class="text-lg font-semibold text-slate-100">
              {{ subsystem }} settings
            </h2>
            <p class="text-xs text-slate-400">ephemeral · in-process only</p>
          </div>
          <button
            type="button"
            class="rounded p-1 text-slate-400 hover:bg-surface-2 hover:text-slate-100"
            @click="$emit('close')"
            aria-label="Close settings"
          >
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M6 6l12 12M6 18L18 6" />
            </svg>
          </button>
        </header>

        <div class="mb-5 rounded border border-amber-700/40 bg-amber-900/20 p-3 text-xs text-amber-200">
          Changes apply to the running process only. A restart restores
          the values from <span class="font-mono">config/</span>. Secrets
          are never editable here.
        </div>

        <slot />
      </aside>
    </transition>
  </Teleport>
</template>

<script setup lang="ts">
defineProps<{
  open: boolean
  subsystem: string
}>()
defineEmits<{ (e: 'close'): void }>()
</script>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 150ms ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
.slide-enter-active,
.slide-leave-active {
  transition: transform 200ms ease;
}
.slide-enter-from,
.slide-leave-to {
  transform: translateX(100%);
}
</style>
