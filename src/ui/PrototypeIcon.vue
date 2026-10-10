<script setup lang="ts">
/** Exact static SVG path map from alice-xxxx.github.io/app.js. No source/user markup enters this component. */
const glyphs = {
  "compass": "<circle cx=\"12\" cy=\"12\" r=\"8.6\"/><path d=\"m15.6 8.4-2.2 5-5 2.2 2.2-5 5-2.2Z\"/><circle cx=\"12\" cy=\"12\" r=\"1\"/>",
  "library": "<path d=\"M4 6.7h5l1.5 1.8H20v9a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6.7Z\"/><path d=\"M4 10h16\"/>",
  "user": "<circle cx=\"12\" cy=\"8.2\" r=\"3.1\"/><path d=\"M5.7 19c.7-3.1 3-4.8 6.3-4.8s5.6 1.7 6.3 4.8\"/>",
  "search": "<circle cx=\"10.7\" cy=\"10.7\" r=\"5.9\"/><path d=\"m15.2 15.2 4.4 4.4\"/>",
  "rss": "<path d=\"M5 5.5a13.5 13.5 0 0 1 13.5 13.5\"/><path d=\"M5 10.5a8.5 8.5 0 0 1 8.5 8.5\"/><circle cx=\"5.2\" cy=\"18.8\" r=\"1.3\" fill=\"currentColor\" stroke=\"none\"/>",
  "history": "<path d=\"M4.5 8.2V4.5h3.7\"/><path d=\"M5.3 6.2A8 8 0 1 1 4.6 16\"/><path d=\"M12 8v4.4l3 1.8\"/>",
  "bookmark": "<path d=\"M7 4.5h10v15l-5-3.2-5 3.2v-15Z\"/>",
  "download": "<path d=\"M12 4v10\"/><path d=\"m8.2 10.4 3.8 3.8 3.8-3.8\"/><path d=\"M5 19h14\"/>",
  "source": "<circle cx=\"12\" cy=\"12\" r=\"2.5\"/><circle cx=\"12\" cy=\"12\" r=\"7.2\"/><path d=\"M12 2.8v2M21.2 12h-2M12 21.2v-2M2.8 12h2\"/>",
  "appearance": "<path d=\"M4 6h16M7 12h10M10 18h4\"/><circle cx=\"8\" cy=\"6\" r=\"1.5\" fill=\"currentColor\" stroke=\"none\"/><circle cx=\"15\" cy=\"12\" r=\"1.5\" fill=\"currentColor\" stroke=\"none\"/><circle cx=\"12\" cy=\"18\" r=\"1.5\" fill=\"currentColor\" stroke=\"none\"/>",
  "backup": "<path d=\"M6.2 8.2A6.7 6.7 0 1 1 5.5 16\"/><path d=\"M4 8.2h4.2V4\"/><path d=\"M12 8v4.5l3 1.7\"/>",
  "settings": "<circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M12 3.5v2M12 18.5v2M3.5 12h2M18.5 12h2M6 6l1.4 1.4M16.6 16.6 18 18M18 6l-1.4 1.4M7.4 16.6 6 18\"/>",
  "chevron-right": "<path d=\"m9 6 6 6-6 6\"/>",
  "close": "<path d=\"m7 7 10 10M17 7 7 17\"/>",
  "more": "<circle cx=\"5\" cy=\"12\" r=\"1\" fill=\"currentColor\" stroke=\"none\"/><circle cx=\"12\" cy=\"12\" r=\"1\" fill=\"currentColor\" stroke=\"none\"/><circle cx=\"19\" cy=\"12\" r=\"1\" fill=\"currentColor\" stroke=\"none\"/>",
  "filter": "<path d=\"M4 6h16M7 12h10M10 18h4\"/>",
  "import": "<path d=\"M12 20V10\"/><path d=\"m8 13.5 4-4 4 4\"/><path d=\"M5 5h14\"/>",
  "export": "<path d=\"M12 4v10\"/><path d=\"m8 8 4-4 4 4\"/><path d=\"M5 19h14\"/>",
  "tune": "<path d=\"M4 7h10M18 7h2M4 17h2M10 17h10\"/><circle cx=\"16\" cy=\"7\" r=\"2\"/><circle cx=\"8\" cy=\"17\" r=\"2\"/>",
  "plus": "<path d=\"M12 5v14M5 12h14\"/>",
  "text": "<path d=\"M6 5h12M12 5v14M8.5 19h7\"/>",
  "book": "<path d=\"M5 4.5h5.5A2.5 2.5 0 0 1 13 7v12H7.5A2.5 2.5 0 0 1 5 16.5v-12Z\"/><path d=\"M19 4.5h-3.5A2.5 2.5 0 0 0 13 7v12h3.5a2.5 2.5 0 0 0 2.5-2.5v-12Z\"/>",
  "image": "<rect x=\"4\" y=\"5\" width=\"16\" height=\"14\" rx=\"2\"/><circle cx=\"9\" cy=\"10\" r=\"1.5\"/><path d=\"m6 17 4.5-4.5 3 3 2-2L19 17\"/>",
  "pdf": "<path d=\"M7 3.5h7l4 4V20H7z\"/><path d=\"M14 3.5V8h4\"/><path d=\"M9.5 14h5M9.5 17h4\"/>",
  "refresh": "<path d=\"M19 7V3.8l-2.2 2.1A7.5 7.5 0 1 0 19 16\"/>",
  "play": "<path d=\"m9 6 9 6-9 6V6Z\"/>",
  "pause": "<path d=\"M9 7v10M15 7v10\"/>",
  "audio": "<path d=\"M9 18V6l9-2v12\"/><circle cx=\"6.5\" cy=\"18\" r=\"2.5\"/><circle cx=\"15.5\" cy=\"16\" r=\"2.5\"/>",
  "debug": "<path d=\"M8 8h8v8a4 4 0 0 1-8 0V8Z\"/><path d=\"M9 5l2 3M15 5l-2 3M5 12h3M16 12h3M5 16h3M16 16h3\"/>",
  "login": "<path d=\"M11 5H6v14h5\"/><path d=\"M10 12h9M16 9l3 3-3 3\"/>",
  "edit": "<path d=\"m5 16-.8 3.8L8 19l9.5-9.5-3-3L5 16Z\"/><path d=\"m13.8 7.2 3 3\"/>",
  "trash": "<path d=\"M5 7h14M9 7V4.5h6V7M7 7l1 13h8l1-13M10 10v6M14 10v6\"/>",
  "info": "<circle cx=\"12\" cy=\"12\" r=\"8.5\"/><path d=\"M12 11v5M12 8h.01\"/>",
  "list": "<path d=\"M9 6h11M9 12h11M9 18h11\"/><circle cx=\"5\" cy=\"6\" r=\"1\" fill=\"currentColor\" stroke=\"none\"/><circle cx=\"5\" cy=\"12\" r=\"1\" fill=\"currentColor\" stroke=\"none\"/><circle cx=\"5\" cy=\"18\" r=\"1\" fill=\"currentColor\" stroke=\"none\"/>",
  "file": "<path d=\"M7 3.5h7l4 4V20H7z\"/><path d=\"M14 3.5V8h4\"/>",
  "rule": "<path d=\"M5 7h14M5 12h9M5 17h14\"/><circle cx=\"17\" cy=\"12\" r=\"2\"/>",
  "layout": "<rect x=\"4\" y=\"4\" width=\"6\" height=\"6\" rx=\"1\"/><rect x=\"14\" y=\"4\" width=\"6\" height=\"6\" rx=\"1\"/><rect x=\"4\" y=\"14\" width=\"6\" height=\"6\" rx=\"1\"/><rect x=\"14\" y=\"14\" width=\"6\" height=\"6\" rx=\"1\"/>",
  "cache": "<path d=\"M6 7c0-2 2.7-3.5 6-3.5S18 5 18 7s-2.7 3.5-6 3.5S6 9 6 7Z\"/><path d=\"M6 7v5c0 2 2.7 3.5 6 3.5s6-1.5 6-3.5V7M6 12v5c0 2 2.7 3.5 6 3.5s6-1.5 6-3.5v-5\"/>",
  "network": "<circle cx=\"12\" cy=\"12\" r=\"8.5\"/><path d=\"M3.8 12h16.4M12 3.5c2.2 2.3 3.2 5.1 3.2 8.5S14.2 18.2 12 20.5C9.8 18.2 8.8 15.4 8.8 12S9.8 5.8 12 3.5Z\"/>",
  "tts": "<path d=\"M5 10v4h3l4 3V7L8 10H5Z\"/><path d=\"M15 9a4 4 0 0 1 0 6M17.5 6.5a7.5 7.5 0 0 1 0 11\"/>",
  "cloud": "<path d=\"M7 18h10a4 4 0 0 0 .5-8A6 6 0 0 0 6.2 8.6 4.7 4.7 0 0 0 7 18Z\"/>",
  "speaker": "<path d=\"M5 10v4h3l4 3V7L8 10H5Z\"/><path d=\"M15 10a3 3 0 0 1 0 4\"/>",
  "link": "<path d=\"m9.5 14.5 5-5\"/><path d=\"M7.2 16.8 5.7 18.3a3 3 0 0 1-4.2-4.2l3.2-3.2a3 3 0 0 1 4.2 0M16.8 7.2l1.5-1.5a3 3 0 0 1 4.2 4.2l-3.2 3.2a3 3 0 0 1-4.2 0\"/>",
  "book-open": "<path d=\"M5 5h5.5A2.5 2.5 0 0 1 13 7.5V19H7.5A2.5 2.5 0 0 1 5 16.5V5Z\"/><path d=\"M19 5h-3.5A2.5 2.5 0 0 0 13 7.5V19h3.5a2.5 2.5 0 0 0 2.5-2.5V5Z\"/>",
  "source-switch": "<path d=\"M7 7h10l-2.5-2.5M17 17H7l2.5 2.5\"/><path d=\"m17 7 2 2-2 2M7 17l-2-2 2-2\"/>",
  "fullscreen": "<path d=\"M8 4H4v4M16 4h4v4M20 16v4h-4M4 16v4h4\"/>",
  "sun": "<circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M18.4 5.6 17 7M7 17l-1.4 1.4\"/>",
  "moon": "<path d=\"M18.2 16.6A7.5 7.5 0 0 1 7.4 5.8 8 8 0 1 0 18.2 16.6Z\"/>",
  "back15": "<path d=\"M7 7H4V4\"/><path d=\"M5 6.5A8 8 0 1 1 4.8 17\"/><path d=\"M10 10v5M13 10.5c2-1 3.5.1 3.5 1.4 0 1.8-3.5 3.1-3.5 3.1h4\"/>",
  "forward15": "<path d=\"M17 7h3V4\"/><path d=\"M19 6.5A8 8 0 1 0 19.2 17\"/><path d=\"M8 10v5M11 10.5c2-1 3.5.1 3.5 1.4 0 1.8-3.5 3.1-3.5 3.1h4\"/>",
  "left": "<path d=\"m14.5 5-7 7 7 7\"/>",
  "right": "<path d=\"m9 6 6 6-6 6\"/>"
} as const;
const props=defineProps<{name:keyof typeof glyphs}>();
</script>
<template>
  <svg class="prototype-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" v-html="glyphs[props.name]"></svg>
</template>
<style scoped>
.prototype-icon {
  display: block;
  width: 22px;
  height: 22px;
  flex: none;
}
</style>
