<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import router, { routeNames } from "../router";
import PrototypeIcon from "./PrototypeIcon.vue";

type MainScreen = "home" | "shelf" | "settings";

const route = useRoute();
const activeScreen = computed(() => route.meta.screen);

const items: { screen: MainScreen; label: string }[] = [
  { screen: "home", label: "发现" },
  { screen: "shelf", label: "内容库" },
  { screen: "settings", label: "我的" },
];

function navigate(screen: MainScreen): void {
  void router.push({ name: routeNames[screen] });
}
</script>

<template>
    <aside class="side-rail">
      <nav class="rail-links" aria-label="Primary navigation">
      <button
        v-for="item in items"
        :key="item.screen"
        type="button"
        :class="['rail-link', { active: activeScreen === item.screen }]"
        :aria-current="activeScreen === item.screen ? 'page' : undefined"
        @click="navigate(item.screen)"
      >
        <span class="glyph">
          <PrototypeIcon :name="item.screen === 'home' ? 'compass' : item.screen === 'shelf' ? 'library' : 'user'" />
        </span>
        <span>{{ item.label }}</span>
      </button>
      </nav>
    </aside>
    <nav class="mobile-nav" aria-label="Primary navigation">
      <div class="mobile-links">
        <button
          v-for="item in items"
          :key="item.screen"
          type="button"
          :class="{ active: activeScreen === item.screen }"
          :aria-current="activeScreen === item.screen ? 'page' : undefined"
          @click="navigate(item.screen)"
        >
          <span class="glyph">
            <PrototypeIcon :name="item.screen === 'home' ? 'compass' : item.screen === 'shelf' ? 'library' : 'user'" />
          </span>
          <span>{{ item.label }}</span>
        </button>
      </div>
    </nav>
</template>

<style scoped>
.side-rail {
  z-index: 20;
  display: flex;
  flex: 0 0 82px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 82px;
  padding: 18px 12px;
  border-right: 1px solid var(--app-line);
  background: rgb(255 255 255 / 92%);
  backdrop-filter: blur(18px);
}

.rail-links {
  display: flex;
  width: 100%;
  flex-direction: column;
  gap: 10px;
}

.rail-link {
  display: flex;
  min-height: 58px;
  width: 100%;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  border: 0;
  border-radius: 16px;
  background: transparent;
  color: var(--app-muted);
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: color .18s ease, background .18s ease;
}

.rail-link:hover,
.rail-link.active {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}

.glyph {
  display: grid;
  width: 25px;
  height: 25px;
  place-items: center;
}

.glyph :deep(svg) {
  width: 24px;
  height: 24px;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.mobile-nav {
  display: none;
}

@media (max-width: 860px) {
  .side-rail {
    display: none;
  }

  .mobile-nav {
    position: fixed;
    z-index: 45;
    inset: auto 0 0;
    display: block;
    width: 100%;
    height: calc(67px + var(--safe-bottom));
    padding: 4px 24px var(--safe-bottom);
    border-top: 1px solid var(--app-line);
    background: rgb(248 248 250 / 92%);
    backdrop-filter: blur(18px);
  }

  .mobile-links {
    display: grid;
    width: 100%;
    height: 100%;
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }

  .mobile-links > button {
    display: flex;
    min-width: 0;
    min-height: 0;
    width: 100%;
    height: 100%;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    padding: 0 8px;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--app-muted);
    font-size: 10px;
    font-weight: 650;
    cursor: pointer;
  }

  .mobile-links > button.active {
    color: var(--app-accent-ink);
  }

  .mobile-links .glyph :deep(svg) {
    width: 23px;
    height: 23px;
  }
}
</style>
