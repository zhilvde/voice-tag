<script setup lang="ts">
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open } from '@tauri-apps/plugin-dialog'
import { ElCheckbox, ElDialog } from 'element-plus'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import Detail from './Detail.vue'
import Icon from './Icon.vue'
import Settings from './Settings.vue'
import { call, clock, errorMessage } from './desktop'
import type { Project, RecordingStatus } from './desktop'

const desktop = isTauri()
const windowHandle = desktop ? getCurrentWindow() : null
const page = ref<'record' | 'history' | 'settings' | 'detail'>('record')
const projects = ref<Project[]>([])
const selected = ref<Project | null>(null)
const recording = ref<RecordingStatus | null>(null)
const busy = ref(false)
const maximized = ref(false)
const notice = ref('')
const noticeError = ref(false)
const search = ref('')
const historyFilter = ref('')
const historySort = ref<'newest' | 'oldest' | 'longest' | 'shortest'>('newest')
const dialog = ref<'delete' | null>(null)
const actionProject = ref<Project | null>(null)
const editingId = ref('')
const nameDraft = ref('')
const nameEditor = ref<HTMLInputElement[]>([])
const deleteAudio = ref(false)
const levels = [6, 11, 23, 15, 39, 25, 51, 33, 20, 32, 14, 19, 7]
const recentProjects = computed(() => projects.value.slice(0, 3))
const filtered = computed(() => {
  const term = search.value.trim().toLowerCase()
  return projects.value.filter(project =>
    (!historyFilter.value || statusKey(project) === historyFilter.value) &&
    project.name.toLowerCase().includes(term),
  ).sort((left, right) => {
    const newestFirst = Date.parse(right.createdAt) - Date.parse(left.createdAt)
    if (historySort.value === 'longest') return right.duration - left.duration || newestFirst
    if (historySort.value === 'shortest') return left.duration - right.duration || newestFirst
    return historySort.value === 'oldest' ? -newestFirst : newestFirst
  })
})
const dialogOpen = computed({ get: () => dialog.value !== null, set: value => { if (!value) dialog.value = null } })
let poll: number | undefined
let noticeTimer: number | undefined

function report(message: string, error = false) {
  notice.value = message
  noticeError.value = error
  window.clearTimeout(noticeTimer)
  if (!error) noticeTimer = window.setTimeout(() => { notice.value = '' }, 4200)
}

async function run(action: () => Promise<void>) {
  if (busy.value) return
  if (!desktop) { report('请在声签 Windows 桌面应用中使用此功能。', true); return }
  busy.value = true
  notice.value = ''
  try { await action() } catch (error) { report(errorMessage(error), true) }
  finally { busy.value = false }
}

async function refreshProjects() {
  projects.value = await call<Project[]>('list_projects')
}

async function navigate(target: 'record' | 'history' | 'settings') {
  page.value = target
  selected.value = null
  editingId.value = ''
  if (desktop && (target === 'history' || target === 'record')) {
    try { await refreshProjects() } catch (error) { report(errorMessage(error), true) }
  }
}

async function showProject(project: Project) {
  selected.value = await call<Project>('get_project', { id: project.id })
  page.value = 'detail'
}

function start() {
  void run(async () => {
    await call('start_recording')
    recording.value = await call<RecordingStatus>('recording_status')
  })
}

function pause() {
  void run(async () => {
    await call('pause_recording', { paused: !recording.value?.paused })
    recording.value = await call<RecordingStatus>('recording_status')
  })
}

function stop() {
  void run(async () => {
    const project = await call<Project>('stop_recording')
    recording.value = null
    await showProject(project)
  })
}

function importAudio() {
  void run(async () => {
    const path = await open({
      title: '导入已有音频文件',
      filters: [{ name: '音频', extensions: ['mp3', 'wav', 'm4a', 'aac', 'flac', 'ogg'] }],
    })
    if (path) await showProject(await call<Project>('import_audio', { path }))
  })
}

async function editName(project: Project) {
  nameDraft.value = project.name
  editingId.value = project.id
  await nextTick()
  nameEditor.value[0]?.focus()
  nameEditor.value[0]?.select()
}

function saveName(project: Project) {
  void run(async () => {
    const name = nameDraft.value.trim()
    if (!name) throw new Error('请输入录音名称。')
    if (name !== project.name) {
      await call('rename_project', { id: project.id, name })
      project.name = name
    }
    editingId.value = ''
    report('录音名称已保存')
  })
}

function confirmDelete(project: Project) {
  actionProject.value = project
  deleteAudio.value = false
  dialog.value = 'delete'
}

function confirm() {
  void run(async () => {
    if (!actionProject.value) return
    await call('delete_project', { id: actionProject.value.id, deleteAudio: deleteAudio.value })
    await refreshProjects()
    dialog.value = null
  })
}

function statusKey(project: Project) {
  return project.recordingStatus === 'failed' ? 'recording-failed' : project.transcriptionStatus
}

function status(project: Project) {
  return { pending: '待转写', processing: '转写中', complete: '已完成', failed: '转写失败', 'recording-failed': '录音失败' }[statusKey(project)]
}

function date(value: string) {
  return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short', hour12: false }).format(new Date(value))
}

async function windowAction(action: 'minimize' | 'maximize' | 'close') {
  if (!windowHandle) return
  try {
    if (action === 'minimize') await windowHandle.minimize()
    else if (action === 'close') await windowHandle.close()
    else {
      await windowHandle.toggleMaximize()
      maximized.value = await windowHandle.isMaximized()
    }
  } catch (error) { report(errorMessage(error), true) }
}

onMounted(() => {
  if (!desktop) return
  void refreshProjects().catch(error => report(errorMessage(error), true))
  let polling = false
  let historyCheckedAt = 0
  poll = window.setInterval(async () => {
    if (busy.value || polling) return
    polling = true
    try {
      const active = recording.value
      if (active) {
        const status = await call<RecordingStatus | null>('recording_status')
        if (!busy.value && recording.value === active) recording.value = status
      }
      if ((page.value === 'history' || page.value === 'record') && Date.now() - historyCheckedAt >= 800 &&
          projects.value.some(project => project.transcriptionStatus === 'processing')) {
        historyCheckedAt = Date.now()
        await refreshProjects()
      }
    } catch (error) {
      report(errorMessage(error), true)
    } finally { polling = false }
  }, 180)
})
onUnmounted(() => { window.clearInterval(poll); window.clearTimeout(noticeTimer) })
</script>

<template>
  <div class="app-window">
    <Detail v-if="page === 'detail' && selected" :key="selected.id" :initial-project="selected"
            @back="navigate('history')" @notice="report" />

    <template v-else>
      <header class="titlebar" data-tauri-drag-region>
        <div class="brand" data-tauri-drag-region><Icon name="wave" :size="23" /><strong>声签</strong></div>
        <div class="window-controls">
          <button class="icon-button" title="最小化" aria-label="最小化" @click="windowAction('minimize')"><Icon name="minimize" /></button>
          <button class="icon-button" :title="maximized ? '还原' : '最大化'" :aria-label="maximized ? '还原' : '最大化'"
                  @click="windowAction('maximize')"><Icon :name="maximized ? 'restore' : 'maximize'" :size="21" /></button>
          <button class="icon-button close-button" title="关闭" aria-label="关闭" @click="windowAction('close')"><Icon name="close" :size="21" /></button>
        </div>
      </header>

      <div class="workspace">
        <nav class="sidebar" aria-label="主导航">
          <button :aria-current="page === 'record' ? 'page' : undefined" @click="navigate('record')"><Icon name="mic" />录音工作台</button>
          <button :aria-current="page === 'history' ? 'page' : undefined" @click="navigate('history')"><Icon name="folder" />历史记录</button>
          <button :aria-current="page === 'settings' ? 'page' : undefined" @click="navigate('settings')"><Icon name="settings" />设置</button>
        </nav>

        <main v-if="page === 'record'" class="workbench page-enter" :class="{ 'workbench-live': recording && !recording.paused && !recording.error }">
          <div class="section-heading"><h1>录音工作台</h1></div>
          <section class="record-deck" aria-label="录音控制">
            <div class="record-wave" :class="{ live: recording && !recording.paused && !recording.error }" aria-hidden="true">
              <i v-for="(height, index) in levels" :key="index" :style="{ height: `${height * (recording && !recording.paused && !recording.error ? 0.65 + recording.level : 1)}px` }" />
            </div>
            <div class="timer" :aria-label="`录音时长 ${clock(recording?.duration ?? 0, true)}`">{{ clock(recording?.duration ?? 0, true) }}</div>
            <p v-if="recording" class="workbench-subtitle" role="status">{{ recording.error || (recording.paused ? '录音已暂停' : '正在录音') }}</p>
            <div class="record-actions">
              <template v-if="!recording">
                <button class="pill record-button" :disabled="busy" @click="start"><Icon name="mic" />开始录音</button>
                <button class="pill import-link" :disabled="busy" @click="importAudio"><Icon name="folder" :size="17" />导入音频文件</button>
              </template>
              <template v-else>
                <button v-if="!recording.error" class="pill import-link" :disabled="busy" @click="pause">
                  <Icon :name="recording.paused ? 'resume' : 'pause'" />{{ recording.paused ? '继续' : '暂停' }}
                </button>
                <button class="pill record-button" :disabled="busy" @click="stop"><Icon name="stop" />结束录音</button>
              </template>
            </div>
          </section>
          <section v-if="recentProjects.length" class="recent-recordings" aria-labelledby="recent-title">
            <div class="section-heading">
              <h2 id="recent-title">最近录音</h2>
              <button class="text-button" @click="navigate('history')">查看全部 <Icon name="next" :size="15" /></button>
            </div>
            <div class="recent-list">
              <button v-for="project in recentProjects" :key="project.id" class="recent-recording"
                      :title="project.name" :disabled="busy" @click="run(() => showProject(project))">
                <span class="recent-icon"><Icon name="wave" /></span>
                <span class="recent-info">
                  <strong>{{ project.name }}</strong>
                  <span>{{ status(project) }} · {{ clock(project.duration) }}</span>
                </span>
                <Icon name="next" :size="16" />
              </button>
            </div>
          </section>
        </main>

        <main v-else-if="page === 'history'" class="history page-enter">
          <div class="section-heading history-heading">
            <div>
              <h1>历史记录</h1>
              <p v-if="projects.length" class="history-count" role="status">{{ filtered.length === projects.length ? `共 ${projects.length} 条录音` : `显示 ${filtered.length} / ${projects.length} 条录音` }}</p>
            </div>
          </div>
          <div v-if="projects.length" class="history-tools">
            <label class="search-box"><Icon name="search" /><input v-model="search" type="search" placeholder="搜索录音名称" aria-label="搜索记录" /></label>
            <div class="history-filters">
              <select v-model="historyFilter" class="form-button" aria-label="筛选转写状态">
                <option value="">全部状态</option><option value="pending">待转写</option><option value="processing">转写中</option>
                <option value="complete">已完成</option><option value="failed">转写失败</option><option value="recording-failed">录音失败</option>
              </select>
              <select v-model="historySort" class="form-button" aria-label="排序历史记录">
                <option value="newest">最新录音</option><option value="oldest">最早录音</option>
                <option value="longest">最长录音</option><option value="shortest">最短录音</option>
              </select>
            </div>
          </div>
          <div v-if="!filtered.length" class="empty-history">
            <Icon name="folder" :size="30" />
            <p>{{ projects.length ? '没有匹配的记录' : '暂无录音记录' }}</p>
            <button v-if="projects.length" class="text-button" @click="search = ''; historyFilter = ''">清除筛选</button>
            <button v-else class="text-button" @click="navigate('record')">前往录音工作台</button>
          </div>
          <table v-else class="history-table">
            <thead><tr>
              <th scope="col">名称</th>
              <th scope="col" :aria-sort="historySort === 'newest' ? 'descending' : historySort === 'oldest' ? 'ascending' : undefined">录制日期</th>
              <th scope="col" :aria-sort="historySort === 'longest' ? 'descending' : historySort === 'shortest' ? 'ascending' : undefined">时长</th>
              <th scope="col">状态</th><th scope="col"><span class="sr-only">操作</span></th>
            </tr></thead>
            <tbody>
              <tr v-for="project in filtered" :key="project.id">
                <td>
                  <form v-if="editingId === project.id" class="history-rename" @submit.prevent="saveName(project)" @keydown.esc="editingId = ''">
                    <input ref="nameEditor" v-model="nameDraft" aria-label="录音名称" maxlength="120" required />
                    <button class="icon-button" type="submit" aria-label="保存名称" title="保存名称" :disabled="busy"><Icon name="check" /></button>
                    <button class="icon-button" type="button" aria-label="取消改名" title="取消改名" :disabled="busy" @click="editingId = ''"><Icon name="close" /></button>
                  </form>
                  <div v-else class="history-name-cell">
                    <button class="history-name" :title="project.name" @click="run(() => showProject(project))"><Icon name="wave" /><span>{{ project.name }}</span></button>
                    <button class="icon-button rename-button" :aria-label="`重命名 ${project.name}`" title="重命名录音" :disabled="busy" @click="editName(project)"><Icon name="edit" :size="16" /></button>
                  </div>
                </td>
                <td>{{ date(project.createdAt) }}</td><td>{{ clock(project.duration) }}</td>
                <td><span class="history-status" :class="`history-status-${project.recordingStatus === 'failed' ? 'failed' : project.transcriptionStatus}`">
                  <span aria-hidden="true"></span>{{ status(project) }}
                  <template v-if="project.transcriptionStatus === 'processing' && project.transcriptionProgress != null"> {{ project.transcriptionProgress }}%</template>
                </span></td>
                <td class="history-actions">
                  <button class="icon-button delete-button" :aria-label="`删除 ${project.name}`" title="删除记录"
                          :disabled="project.transcriptionStatus === 'processing'" @click="confirmDelete(project)"><Icon name="trash" :size="17" /></button>
                </td>
              </tr>
            </tbody>
          </table>
        </main>
        <Settings v-else-if="page === 'settings'" @notice="report" />
      </div>
    </template>

    <div v-if="notice" class="notice" :class="{ 'notice-error': noticeError }" :role="noticeError ? 'alert' : 'status'">
      <span>{{ notice }}</span><button class="icon-button" aria-label="关闭提示" @click="notice = ''"><Icon name="close" :size="16" /></button>
    </div>

    <ElDialog v-model="dialogOpen" title="删除记录"
              width="min(520px, calc(100vw - 48px))" append-to-body align-center :close-on-click-modal="!busy">
      <form class="dialog-form" @submit.prevent="confirm">
        <p>确定删除「{{ actionProject?.name }}」的记录及转写稿？</p>
        <ElCheckbox v-model="deleteAudio">同时删除声签保存的原始音频</ElCheckbox>
        <div class="dialog-actions">
          <button type="button" class="form-button" :disabled="busy" @click="dialog = null">取消</button>
          <button type="submit" class="form-button primary" :disabled="busy">删除记录</button>
        </div>
      </form>
    </ElDialog>
  </div>
</template>
