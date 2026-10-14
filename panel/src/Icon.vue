<script setup lang="ts">
withDefaults(defineProps<{ name: string; size?: number }>(), { size: 20 });
const paths: Record<string, string> = {
  home: "M3 10 12 3l9 7v10H3Z M9 20v-7h6v7",
  settings: "M4 6h16M4 12h16M4 18h16M8 3v6M16 9v6M10 15v6",
  journal: "M6 3h13v18H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3Z M8 8h7M8 12h7M8 16h4",
  skin: "M8 3 3 5l-2 6 5 2v8h12v-8l5-2-2-6-5-2c0 5-8 5-8 0Z",
  clipboard: "M8 5H5v16h14V5h-3M8 3h8v5H8Z M8 12h8M8 16h5",
  transfer: "M3 12h5l2 3h4l2-3h5v8H3Z M12 3v9M8 8l4 4 4-4",
  heart: "M12 21 3 12C-3 3 8-1 12 6c4-7 15-3 9 6Z",
  sparkles: "M12 2 15 9l7 3-7 3-3 7-3-7-7-3 7-3Z",
  play: "M8 4 20 12 8 20Z",
  moon: "M20 14A9 9 0 0 1 10 3a9 9 0 1 0 10 11Z",
  search: "M21 21l-6-6M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0",
  file: "M6 3h8l5 5v13H6Z M14 3v6h5",
  image: "M3 3h18v18H3Z M3 17l6-6 5 5 3-3 4 4M8 7h.01",
  text: "M4 5h16M12 5v16M8 21h8",
  link: "M9 15l6-6M8 16l-2 2a4 4 0 0 1-6-6l4-4M16 8l2-2a4 4 0 0 1 6 6l-4 4",
  star: "m12 2 3 6 7 1-5 5 1 7-6-3-6 3 1-7-5-5 7-1Z",
  trash: "M3 6h18M8 6V3h8v3M6 6l1 15h10l1-15M10 10v7M14 10v7",
  copy: "M8 8h13v13H8Z M16 8V3H3v13h5",
  plus: "M12 4v16M4 12h16",
  folder: "M3 5h7l2 3h9v12H3Z",
  arrow: "M4 12h16M14 6l6 6-6 6",
  check: "m4 12 5 5L20 6",
  close: "m5 5 14 14M19 5 5 19",
  refresh: "M20 10A8 8 0 1 0 20 16M20 3v7h-7",
  power: "M12 2v10M6 5a9 9 0 1 0 12 0",
  shield: "M12 2 3 6v6c0 6 9 10 9 10s9-4 9-10V6Z M8 12l3 3 5-5",
  grip: "M8 5h.01M16 5h.01M8 12h.01M16 12h.01M8 19h.01M16 19h.01",
  pause: "M8 4v16M16 4v16",
};
</script>
<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path :d="paths[name] || paths.sparkles" />
  </svg>
</template>
