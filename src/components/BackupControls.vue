<script setup lang="ts">
const props = defineProps<{ busy?: "create" | "restore" | null; lastBackupAt?: number | null }>();
const emit = defineEmits<{ create: []; restore: [] }>();

const lastBackup = () => props.lastBackupAt ? new Date(props.lastBackupAt).toLocaleString() : "尚未备份";
</script>

<template>
  <section class="backup-card">
    <div class="backup-symbol" aria-hidden="true">↗</div>
    <div class="backup-copy"><p class="eyebrow">数据管理</p><h3>备份与恢复</h3><p>备份包含书架、阅读记录、书签、设置、缓存内容、书源和登录状态。</p><small>请把备份文件保存在可信位置。最近备份：{{ lastBackup() }}</small></div>
    <div class="backup-actions"><button class="backup-create" :disabled="!!busy" @click="emit('create')">{{ busy === 'create' ? '正在保存…' : '创建备份' }}</button><button class="backup-restore" :disabled="!!busy" @click="emit('restore')">{{ busy === 'restore' ? '正在恢复…' : '从备份恢复' }}</button></div>
    <p class="restore-note">恢复会用所选备份替换当前本地数据，并在完成后重新载入书架。</p>
  </section>
</template>

<style scoped>
.backup-card { display:grid; grid-template-columns:43px minmax(0,1fr) auto; align-items:center; gap:13px; padding:19px 21px 17px; border:1px solid #e7ebe7; border-radius:13px; background:#fff; }
.backup-symbol { width:42px; height:42px; display:grid; place-items:center; border-radius:12px; color:#5d8068; background:#eaf2ec; font-size:23px; }
.backup-copy .eyebrow { margin:0 0 3px; color:#657c6a; font-size:14px; font-weight:800; }.backup-copy h3 { margin:0; color:#35453a; font-size:16px; }.backup-copy p { margin:5px 0 4px; color:#56655b; font-size:14px; line-height:1.6; }.backup-copy small { color:#66746a; font-size:14px; line-height:1.5; }
.backup-actions { display:flex; gap:7px; }.backup-actions button { min-height:44px; padding:0 13px; border:1px solid #d2dfd3; border-radius:7px; color:#466851; background:#f4f8f4; font-size:14px; font-weight:700; cursor:pointer; }.backup-actions .backup-create { color:#fff; border-color:#3c725f; background:#3c725f; }.backup-actions button:hover:not(:disabled) { filter:brightness(.97); }.backup-actions button:disabled { opacity:.5; cursor:not-allowed; }
.restore-note { grid-column:2 / -1; margin:0; color:#855f3e; font-size:14px; line-height:1.5; }
@media(max-width:680px) { .backup-card { grid-template-columns:36px minmax(0,1fr); padding:15px 13px; }.backup-symbol { width:35px; height:35px; font-size:19px; }.backup-actions { grid-column:1 / -1; }.backup-actions button { flex:1; }.restore-note { grid-column:1 / -1; } }
</style>
