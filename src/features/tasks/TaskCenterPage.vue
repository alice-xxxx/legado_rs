<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import BackButton from "../../ui/BackButton.vue";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import { clearFinishedTasks } from "../../api/tasks";
import { shelfBooks } from "../shelf/shelfBooks";
import { tasks, tasksLoading, tasksError, refreshTasks, ensureTasksLoaded } from "./tasksStore";
import { notify, errorText } from "../../app/notifications";
import { backupBusy } from "../settings/backupState";

const clearing = ref(false);
const titles = computed(() => new Map(shelfBooks.value.books.map(book => [book.id, book.title])));
async function clearHistory(): Promise<void> {
  if (clearing.value || backupBusy.value) return;
  clearing.value = true;
  try {
    await clearFinishedTasks();
    await refreshTasks();
  } catch (error) { notify(errorText(error), "error"); }
  finally { clearing.value = false; }
}
onMounted(() => { void ensureTasksLoaded().catch(error => notify(errorText(error), "error")); });
</script>

<template>
  <div class="tasks-workspace">
    <header class="tasks-topbar safe-area-top">
      <BackButton label="返回我的" @click="returnToRoute(routeNames.settings)" />
      <strong>历史记录</strong>
      <button class="text-button" :disabled="clearing || !!backupBusy || !tasks.length" @click="clearHistory">清空</button>
    </header>
    <main class="task-center">
      <p v-if="tasksError" class="error">{{ tasksError }}</p>
      <p v-if="tasksLoading" class="muted">正在读取…</p>
      <p v-else-if="!tasks.length" class="muted">暂无历史记录</p>
      <article v-for="task in tasks" :key="task.id" class="task-card">
        <strong>{{ (task.bookId && titles.get(task.bookId)) || task.keyword || '书籍缓存' }}</strong>
        <span class="muted">{{ task.status === 'completed' ? '已完成' : task.status === 'failed' ? '失败' : '已取消' }} · {{ task.completed }} / {{ task.total }}</span>
        <p v-if="task.error" class="error">{{ task.error }}</p>
      </article>
    </main>
  </div>
</template>

<style scoped>
.tasks-workspace{min-height:100%;background:var(--app-bg)}
.tasks-topbar{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:16px;border-bottom:1px solid var(--app-line)}
.task-center{padding:16px}
.task-card{display:flex;flex-direction:column;gap:6px;padding:16px 0;border-bottom:1px solid var(--app-line)}
.task-card p{margin:0}
</style>
