<script setup lang="ts">
defineProps<{ name: string; size?: number }>()
</script>

<template>
  <svg :width="size ?? 18" :height="size ?? 18" viewBox="0 0 24 24" fill="none" stroke="currentColor"
       stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path v-if="name === 'wave'" d="M2.8 10.5v3m3.7-7v11m3.7-14v17m3.7-13v9m3.7-11v13m3.7-9v5" stroke-width="1.85"/>
    <template v-else-if="name === 'mic'">
      <rect x="9" y="2.7" width="6" height="11" rx="3"/>
      <path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5v3.8m-2.7 0h5.4"/>
    </template>
    <template v-else-if="name === 'folder'">
      <path d="M3 7.1a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8.8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM3 10.6h18"/>
    </template>
    <template v-else-if="name === 'settings'">
      <path d="m10.2 3-.7 2.2-1.9 1.1-2.3-.5-1.8 3.1 1.6 1.7v2.3l-1.6 1.7 1.8 3.1 2.3-.5 1.9 1.1.7 2.2h3.6l.7-2.2 1.9-1.1 2.3.5 1.8-3.1-1.6-1.7v-2.3l1.6-1.7-1.8-3.1-2.3.5-1.9-1.1-.7-2.2z"/>
      <circle cx="12" cy="11.8" r="2.8"/>
    </template>
    <path v-else-if="name === 'minimize'" d="M6 12h12"/>
    <rect v-else-if="name === 'maximize'" x="4" y="4" width="16" height="16" rx="2.4"/>
    <path v-else-if="name === 'restore'" d="M8 4h10a2 2 0 0 1 2 2v10m-14-8h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2z"/>
    <path v-else-if="name === 'close'" d="m6 6 12 12M18 6 6 18"/>
    <path v-else-if="name === 'back'" d="M19 12H5m6-6-6 6 6 6"/>
    <path v-else-if="name === 'next'" d="m9 6 6 6-6 6"/>
    <template v-else-if="name === 'skip-back' || name === 'skip-next'">
      <path v-if="name === 'skip-back'" d="M4 11a8 8 0 1 1 2 6M4 6v5h5"/>
      <path v-else d="M20 11a8 8 0 1 0-2 6m2-11v5h-5"/>
      <path d="M14 9h-3l-.5 2.5h2a1.75 1.75 0 0 1 0 3.5H10"/>
    </template>
    <path v-else-if="name === 'edit'" d="m5.1 14.8 9.9-9.9a2.7 2.7 0 0 1 3.8 3.8L8.9 18.6l-4.6 1zM13.3 6.6l3.8 3.8"/>
    <path v-else-if="name === 'download'" d="M12 3.5v11m-4-4 4 4 4-4m4 3v3.2a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-3.2"/>
    <template v-else-if="name === 'search'">
      <circle cx="10.7" cy="10.7" r="6.7"/><path d="m15.4 15.4 4.4 4.4"/>
    </template>
    <template v-else-if="name === 'play' || name === 'playing'">
      <circle cx="12" cy="12" r="11" fill="currentColor" stroke="none"/>
      <path v-if="name === 'play'" fill="var(--surface)" stroke="none" d="M10 7.6 16.2 12 10 16.4z"/>
      <path v-else stroke="var(--surface)" stroke-width="2" d="M10 8.8v6.4m4-6.4v6.4"/>
    </template>
    <path v-else-if="name === 'pause'" d="M9 5v14m6-14v14" stroke-width="2.5"/>
    <path v-else-if="name === 'resume'" d="m8 5 11 7-11 7z" fill="currentColor" stroke="none"/>
    <rect v-else-if="name === 'stop'" x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor" stroke="none"/>
    <path v-else-if="name === 'trash'" d="M4 6h16m-14 0 1 13a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-13M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m-5 4v7m4-7v7"/>
    <path v-else-if="name === 'add'" d="M5 12h14m-7-7v14" />
    <path v-else-if="name === 'copy'" d="M8 16H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v3m-5 0h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-8a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2Z" />
    <path v-else-if="name === 'check'" d="m5 12 4 4L19 6"/>
  </svg>
</template>
