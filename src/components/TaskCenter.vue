<script setup lang="ts">
export interface AppTask {
  id: string;
  kind: string;
  status: string;
  bookId?: string;
  sourceIds?: string[];
  keyword?: string;
  page: number;
  fromIndex: number;
  total: number;
  completed: number;
  createdAtMs: number;
  updatedAtMs: number;
  error?: string;
  result?: {
    bookResourceId?: string;
    addedCount?: number;
    movedProgress?: boolean;
    committed?: boolean;
  };
}

const props = defineProps<{ tasks: AppTask[]; bookTitles: Record<string, string> }>();
const emit = defineEmits<{ command: [action: "pause" | "resume" | "cancel", taskId: string] }>();

function label(task: AppTask): string {
  if (task.kind === "search") return task.keyword ? `搜索“${task.keyword}”` : "书籍搜索";
  const book = task.bookId ? props.bookTitles[task.bookId] ?? "书籍" : "书籍";
  if (task.kind === "chapterDownload") return `准备《${book}》的章节`;
  if (task.kind === "refreshChapters") return `更新《${book}》的目录`;
  if (task.kind === "checkNewChapters") return `检查《${book}》的新章节`;
  return "阅读任务";
}

function statusLabel(status: string): string {
  return ({ queued: "等待中", running: "进行中", pausing: "正在暂停", paused: "已暂停", cancelling: "正在取消", cancelled: "已取消", completed: "已完成", failed: "失败", interrupted: "上次未完成" } as Record<string, string>)[status] ?? status;
}

function progress(task: AppTask): number {
  if (task.status === "completed") return 100;
  if (task.total <= 0) return task.status === "running" ? 8 : 0;
  return Math.max(0, Math.min(100, Math.round(task.completed / task.total * 100)));
}

const activeStatuses = new Set(["queued", "running", "pausing", "paused", "cancelling"]);
</script>

<template>
  <section class="task-center">
    <header class="task-center-heading"><div><p class="eyebrow">下载与搜索</p><h3>任务</h3></div><span class="task-count">{{ tasks.filter((task) => activeStatuses.has(task.status)).length }} 个进行中</span></header>
    <div v-if="tasks.length" class="task-list">
      <article v-for="task in [...tasks].sort((a,b) => b.updatedAtMs - a.updatedAtMs)" :key="task.id" class="task-card" :class="`task-${task.status}`">
        <div class="task-card-top"><div class="task-card-copy"><strong>{{ label(task) }}</strong><span>{{ statusLabel(task.status) }}<template v-if="task.total"> · {{ task.completed }} / {{ task.total }}</template></span></div><div class="task-actions">
          <button v-if="task.status === 'running' || task.status === 'queued'" :aria-label="`暂停${label(task)}`" @click="emit('command','pause',task.id)">暂停</button>
          <button v-else-if="task.status === 'paused'" :aria-label="`继续${label(task)}`" @click="emit('command','resume',task.id)">继续</button>
          <button v-if="activeStatuses.has(task.status)" class="cancel-task" :aria-label="`取消${label(task)}`" @click="emit('command','cancel',task.id)">取消</button>
        </div></div>
        <div class="task-track"><span :class="{ busy: task.status === 'running' && !task.total }" :style="{ width: `${progress(task)}%` }"></span></div>
        <p v-if="task.error" class="task-error">{{ task.error }}</p>
      </article>
    </div>
    <div v-else class="task-empty">还没有搜索或下载任务。</div>
  </section>
</template>

<style scoped>
.task-center { padding:21px; border:1px solid #e7ebe7; border-radius:13px; background:#fff; }
.task-center-heading { display:flex; align-items:center; justify-content:space-between; gap:10px; margin-bottom:12px; }.task-center-heading .eyebrow { margin:0 0 4px; color:#62776a; font-size:14px; }.task-center-heading h3 { margin:0; color:#35453a; font-size:17px; }.task-count { padding:6px 9px; border-radius:12px; color:#476650; background:#edf4ef; font-size:14px; }
.task-list { display:grid; gap:8px; }.task-card { padding:12px; border:1px solid #e0e7e0; border-radius:9px; background:#fcfdfb; }.task-card-top { display:flex; align-items:center; justify-content:space-between; gap:10px; }.task-card-copy { min-width:0; display:flex; flex-direction:column; gap:5px; }.task-card-copy strong { overflow:hidden; color:#46564b; font-size:14px; text-overflow:ellipsis; white-space:nowrap; }.task-card-copy > span { color:#5d6b61; font-size:14px; }.task-actions { flex:none; display:flex; gap:5px; }.task-actions button { min-height:44px; padding:0 10px; border:0; border-radius:5px; color:#41654b; background:#edf4ef; font-size:14px; cursor:pointer; }.task-actions .cancel-task { color:#9b4b42; background:#f8eeec; }.task-track { height:4px; overflow:hidden; margin-top:11px; border-radius:4px; background:#edf0ed; }.task-track span { position:relative; display:block; height:100%; border-radius:inherit; background:#6d9d7a; transition:width .2s; }.task-track span.busy { width:35%!important; animation:task-slide 1.2s ease-in-out infinite alternate; }.task-error { margin:8px 0 0; color:#963f36; font-size:14px; line-height:1.5; }
.task-empty { display:grid; min-height:100px; place-items:center; color:#66746a; font-size:14px; }.task-completed .task-track span { background:#8ba790; }.task-failed .task-track span { background:#b96a60; }
@keyframes task-slide { from { transform:translateX(-15%); } to { transform:translateX(190%); } }
@media(max-width:760px) { .task-center { padding:15px 13px; } }
</style>
