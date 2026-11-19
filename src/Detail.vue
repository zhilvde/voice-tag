<script setup lang="ts">
import { convertFileSrc } from '@tauri-apps/api/core'
import { save } from '@tauri-apps/plugin-dialog'
import { ElCheckbox, ElDialog } from 'element-plus'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import Icon from './Icon.vue'
import { call, clock, errorMessage, methodLabels, onlineRouteDescription } from './desktop'
import type { Preferences, Project, ReplacementRule, Segment } from './desktop'

const props = defineProps<{ initialProject: Project }>()
const emit = defineEmits<{ back: []; notice: [message: string, error?: boolean] }>()
const project = ref(props.initialProject)
const language = ref(project.value.language)
const audio = ref<HTMLAudioElement>()
const audioUrl = ref('')
const audioError = ref('')
const position = ref(0)
const duration = ref(project.value.duration)
const playing = ref(false)
const rate = ref(1)
const busy = ref(false)
const editing = ref(false)
const focused = ref('')
const query = ref('')
const searchMatchId = ref('')
const saveError = ref('')
const renamingSpeakers = ref(false)
const followPlayback = ref(true)
const speakerFilter = ref('')
const splitSegment = ref<Segment>()
const splitDialog = ref(false)
const splitOffset = ref(0)
const splitTime = ref<number | null>(null)
const splitEnd = computed(() => {
  const segment = splitSegment.value
  if (!segment) return 0
  return segment.end > segment.start ? segment.end :
    orderedSegments.value.find(item => item.start > segment.start)?.start ?? duration.value
})
const splitParts = computed(() => {
  const text = splitSegment.value?.text ?? ''
  return [text.slice(0, splitOffset.value).trim(), text.slice(splitOffset.value).trim()] as const
})
const canSplit = computed(() => Boolean(splitSegment.value && splitParts.value.every(Boolean) &&
  typeof splitTime.value === 'number' && splitTime.value > splitSegment.value.start && splitTime.value < splitEnd.value))
const speakerNames = ref<Record<string, string>>({})
const exportDialog = ref(false)
const format = ref<'txt' | 'md'>('txt')
const timestamps = ref(true)
const speakerLabels = ref(true)
const merge = ref(false)
const selectExport = ref(false)
const selected = ref<string[]>([])
const status = computed(() => project.value.transcriptionStatus)
const method = ref<Preferences['method']>('local')
const apiModel = ref('')
const preferencesLoaded = ref(false)
const globalRules = ref<ReplacementRule[]>([])
const replaceDialog = ref(false)
const replaceMode = ref<'single' | 'global'>('single')
const findText = ref('')
const replacementText = ref('')
const replacePreview = computed(() => {
  if (!replaceDialog.value) return []
  const rules = replaceMode.value === 'single'
    ? [{ find: findText.value, replacement: replacementText.value }]
    : globalRules.value
  return project.value.segments.flatMap(segment => {
    let text = segment.text
    for (const rule of rules) {
      if (rule.find) text = text.replaceAll(rule.find, () => rule.replacement)
    }
    return text === segment.text ? [] : [{ segment, text }]
  })
})
const knownSpeakers = computed(() => [...new Set(['A', 'B', ...project.value.speakers.map(speaker => speaker.id)])].filter(id => id !== 'unknown').map(id => ({
  id,
  name: project.value.speakers.find(speaker => speaker.id === id)?.name ?? `说话人 ${id}`,
})))
const shownSegments = computed(() => {
  const term = query.value.trim().toLowerCase()
  return orderedSegments.value.filter(segment => (!speakerFilter.value || segment.speakerId === speakerFilter.value) &&
    (!term || segment.text.toLowerCase().includes(term) || speakerName(segment.speakerId).toLowerCase().includes(term)))
})
const searchMatchIndex = computed(() => shownSegments.value.findIndex(segment => segment.id === searchMatchId.value))
const hasCopyableText = computed(() => project.value.segments.some(segment => segment.text.trim()))
const speakerCounts = computed(() => {
  const counts = new Map<string, number>()
  for (const segment of project.value.segments) counts.set(segment.speakerId, (counts.get(segment.speakerId) ?? 0) + 1)
  return counts
})
const hasFilter = computed(() => Boolean(query.value.trim() || speakerFilter.value))
const playbackProgress = computed(() => duration.value > 0 ? position.value / duration.value * 100 : 0)
const orderedSegments = computed(() => [...project.value.segments].sort((left, right) => left.start - right.start))
const dialogueTurns = computed(() => {
  const shownIds = new Set(shownSegments.value.map(segment => segment.id))
  const turns: { id: string; speakerId: string; segments: Segment[] }[] = []
  let previous: Segment | undefined
  for (const segment of orderedSegments.value) {
    if (!shownIds.has(segment.id)) {
      previous = undefined
      continue
    }
    const lastTurn = turns.at(-1)
    if (lastTurn && previous?.speakerId === segment.speakerId && segment.speakerId !== 'unknown' &&
        !previous.overlap && !segment.overlap) {
      lastTurn.segments.push(segment)
    } else {
      turns.push({ id: segment.id, speakerId: segment.speakerId, segments: [segment] })
    }
    previous = segment
  }
  return turns
})
const activeSegmentId = computed(() => {
  const time = position.value
  let previous = ''
  for (let index = orderedSegments.value.length - 1; index >= 0; index--) {
    const segment = orderedSegments.value[index]!
    if (segment.start > time) continue
    previous ||= segment.id
    if (time < segment.end) return segment.id
  }
  return previous
})
let poll: number | undefined
let currentSave = Promise.resolve()
let revision = 0

function speakerName(id: string) {
  return project.value.speakers.find(speaker => speaker.id === id)?.name ??
    (id === 'unknown' ? '待确认' : `说话人 ${id}`)
}

function speakerSide(id: string) {
  if (id === 'unknown') return 'dialogue-unassigned'
  return knownSpeakers.value.findIndex(speaker => speaker.id === id) % 2 === 1 ? 'dialogue-right' : 'dialogue-left'
}

function speakerInitial(id: string) {
  if (id === 'unknown') return '?'
  const name = speakerName(id)
  return name === `说话人 ${id}` ? id : Array.from(name.trim())[0] ?? id
}

async function run(action: () => Promise<void>) {
  if (busy.value) return
  busy.value = true
  try { await action() } catch (error) { emit('notice', errorMessage(error), true) }
  finally { busy.value = false }
}

async function refresh() {
  project.value = await call<Project>('get_project', { id: project.value.id })
}

onMounted(async () => {
  try {
    const path = await call<string>('audio_path', { id: project.value.id })
    audioUrl.value = convertFileSrc(path)
  } catch (error) { audioError.value = errorMessage(error) }
  try {
    const preferences = await call<Preferences>('preferences')
    method.value = preferences.method
    apiModel.value = preferences.apiModel
    globalRules.value = preferences.replacementRules
    preferencesLoaded.value = true
  }
  catch (error) { emit('notice', errorMessage(error), true) }
  let polling = false
  poll = window.setInterval(async () => {
    if (status.value !== 'processing' || polling || busy.value) return
    polling = true
    try { await refresh() } catch (error) { emit('notice', errorMessage(error), true) }
    finally { polling = false }
  }, 800)
})
onUnmounted(() => window.clearInterval(poll))

function loadedAudio() {
  const actual = audio.value?.duration
  if (!actual || !Number.isFinite(actual)) return
  duration.value = actual
  if (Math.abs(actual - project.value.duration) > 0.25) {
    void call('update_duration', { id: project.value.id, duration: actual }).catch(error => emit('notice', errorMessage(error), true))
  }
}

function seek(seconds: number, reveal = false) {
  if (!audio.value || !Number.isFinite(audio.value.duration)) return
  audio.value.currentTime = Math.max(0, Math.min(seconds, audio.value.duration))
  position.value = audio.value.currentTime
  if (reveal && activeSegmentId.value) {
    const id = activeSegmentId.value
    if (!shownSegments.value.some(segment => segment.id === id)) {
      query.value = ''
      speakerFilter.value = ''
    }
    void nextTick(() => {
      if (activeSegmentId.value === id) document.getElementById(`cue-${id}`)?.scrollIntoView({ block: 'nearest' })
    })
  }
}

watch(activeSegmentId, id => {
  if (id && playing.value && followPlayback.value && !editing.value && !focused.value && !renamingSpeakers.value && searchMatchIndex.value < 0) {
    document.getElementById(`cue-${id}`)?.scrollIntoView({ block: 'nearest' })
  }
}, { flush: 'post' })

watch([query, speakerFilter], () => { searchMatchId.value = '' })

async function moveSearchMatch(step: number) {
  const matches = shownSegments.value
  if (!query.value.trim() || !matches.length) return
  const current = searchMatchIndex.value
  const index = current < 0 ? (step < 0 ? matches.length - 1 : 0) : (current + step + matches.length) % matches.length
  const id = matches[index]!.id
  searchMatchId.value = id
  await nextTick()
  document.getElementById(`cue-${id}`)?.scrollIntoView({ block: 'nearest' })
}

function searchKey(event: KeyboardEvent) {
  if (event.isComposing) return
  event.preventDefault()
  void moveSearchMatch(event.shiftKey ? -1 : 1)
}

async function togglePlayback() {
  if (!audio.value) return
  if (!audio.value.paused) audio.value.pause()
  else {
    try { await audio.value.play() } catch (error) { audioError.value = errorMessage(error) }
  }
}

function changeRate() {
  if (audio.value) audio.value.playbackRate = rate.value
}

function startTranscription() {
  void run(async () => {
    await call('start_transcription', { id: project.value.id, language: language.value })
    await refresh()
  })
}

function startManual() {
  void run(async () => {
    project.value = await call<Project>('start_manual_transcript', { id: project.value.id })
    editing.value = true
    await addSegment()
  })
}

async function addSegment() {
  editing.value = true
  const start = position.value
  const id = crypto.randomUUID()
  const segment: Segment = { id, start, end: start, text: '', speakerId: 'unknown', overlap: false }
  const index = project.value.segments.findIndex(item => item.start > start)
  project.value.segments.splice(index < 0 ? project.value.segments.length : index, 0, segment)
  query.value = ''
  speakerFilter.value = ''
  await nextTick()
  document.getElementById(`segment-${id}`)?.focus()
}

function removeBlankSegment(id: string) {
  project.value.segments = project.value.segments.filter(segment => segment.id !== id)
  persist()
}

function showReplace() {
  findText.value = ''
  replacementText.value = ''
  replaceMode.value = 'single'
  replaceDialog.value = true
}

async function applyReplacement() {
  const changes = replacePreview.value
  if (!changes.length) return
  for (const { segment, text } of changes) segment.text = text
  persist()
  replaceDialog.value = false
  await currentSave
  if (!saveError.value) emit('notice', `已更新 ${changes.length} 个片段`)
}

function persist() {
  const projectId = project.value.id
  const version = ++revision
  currentSave = currentSave.then(async () => {
    if (version !== revision) return
    const speakers = project.value.speakers.map(speaker => ({ ...speaker }))
    const segments = project.value.segments.map(segment => ({ ...segment }))
    try {
      await call('save_transcript', { id: projectId, speakers, segments })
      saveError.value = ''
    } catch (error) { saveError.value = errorMessage(error) }
  })
}

function changeSpeaker(segment: Segment, event: Event) {
  const id = (event.target as HTMLSelectElement).value
  const speaker = knownSpeakers.value.find(item => item.id === id)
  if (speaker && !project.value.speakers.some(item => item.id === id)) project.value.speakers.push(speaker)
  segment.speakerId = id
  persist()
}

function showSplit(segment: Segment) {
  splitSegment.value = segment
  splitOffset.value = 0
  splitTime.value = null
  splitDialog.value = true
}

function pickSplitPoint(event: Event) {
  const field = event.target as HTMLTextAreaElement
  const segment = splitSegment.value
  if (!segment || field.selectionStart === splitOffset.value) return
  splitOffset.value = field.selectionStart
  const estimate = segment.start + (splitEnd.value - segment.start) * splitOffset.value / segment.text.length
  splitTime.value = Number(estimate.toFixed(3))
}

function applySplit() {
  const segment = splitSegment.value
  const time = splitTime.value
  if (!segment || time === null || !canSplit.value) return
  const index = project.value.segments.findIndex(item => item.id === segment.id)
  if (index < 0) return
  const [first, second] = splitParts.value
  const id = crypto.randomUUID()
  project.value.segments.splice(index, 1,
    { ...segment, end: time, text: first },
    { ...segment, id, start: time, end: splitEnd.value, text: second })
  splitDialog.value = false
  editing.value = true
  query.value = ''
  speakerFilter.value = ''
  persist()
  seek(time, true)
}

async function renameSpeakers(id?: string) {
  if (!renamingSpeakers.value) {
    speakerNames.value = Object.fromEntries(knownSpeakers.value.map(speaker => [speaker.id, speaker.name]))
    renamingSpeakers.value = true
  }
  await nextTick()
  const input = document.getElementById(`speaker-name-${id ?? 'A'}`) as HTMLInputElement | null
  input?.focus()
  input?.select()
}

async function saveSpeakerNames() {
  for (const { id } of knownSpeakers.value) {
    const name = speakerNames.value[id]?.trim()
    if (!name) return
  }
  const named = knownSpeakers.value.map(speaker => ({ id: speaker.id, name: speakerNames.value[speaker.id]!.trim() }))
  project.value.speakers = [...named, ...project.value.speakers.filter(speaker => speaker.id === 'unknown')]
  persist()
  renamingSpeakers.value = false
  await currentSave
  if (!saveError.value) emit('notice', '说话人姓名已保存')
}

async function beginEditing(segment: Segment, event: MouseEvent) {
  seek(segment.start)
  const row = (event.currentTarget as HTMLElement).parentElement
  focused.value = segment.id
  await nextTick()
  row?.querySelector('textarea')?.focus()
}

async function back() {
  await currentSave
  if (!saveError.value) emit('back')
}

async function writeClipboard(text: string) {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text)
      return
    } catch {
      // WebView may restrict the Clipboard API while allowing the document copy command.
    }
  }
  const active = document.activeElement
  const field = document.createElement('textarea')
  field.value = text
  field.readOnly = true
  field.style.cssText = 'position:fixed;left:0;top:0;width:1px;height:1px;opacity:0;pointer-events:none'
  document.body.append(field)
  try {
    field.select()
    if (!document.execCommand('copy')) throw new Error('无法访问剪贴板。')
  } finally {
    field.remove()
    if (active instanceof HTMLElement) active.focus({ preventScroll: true })
  }
}

async function copyTranscript(segment?: Segment) {
  try {
    if (segment) {
      await writeClipboard(segment.text)
      emit('notice', '已复制片段')
      return
    }
    const lines = orderedSegments.value.filter(item => item.text.trim()).map(item =>
      `[${clock(item.start)}] ${speakerName(item.speakerId)}：${item.text.trim()}`)
    if (!lines.length) return
    await writeClipboard(`${project.value.name}\n\n${lines.join('\n\n')}`)
    emit('notice', '已复制转写全文')
  } catch (error) {
    emit('notice', errorMessage(error), true)
  }
}

function showExport() {
  selectExport.value = false
  selected.value = project.value.segments.map(segment => segment.id)
  exportDialog.value = true
}

function exportFile() {
  void run(async () => {
    await currentSave
    if (saveError.value) throw new Error(saveError.value)
    const name = project.value.name.replace(/[<>:"/\\|?*]/g, '_').replace(/[. ]+$/, '') || '声签记录'
    const path = await save({
      title: '保存转写文件',
      defaultPath: `${name}.${format.value}`,
      filters: [{ name: format.value === 'txt' ? '文本文件' : 'Markdown', extensions: [format.value] }],
    })
    if (!path) return
    await call('export_transcript', {
      id: project.value.id,
      options: { path, format: format.value, timestamps: timestamps.value, speakerLabels: speakerLabels.value,
        merge: merge.value, selection: selectExport.value ? selected.value : null },
    })
    exportDialog.value = false
    emit('notice', '已导出转写文件')
  })
}
</script>

<template>
  <main class="detail">
    <header class="detail-header" data-tauri-drag-region>
      <button class="icon-button" title="返回历史记录" aria-label="返回历史记录" @click="back"><Icon name="back" /></button>
      <h1 data-tauri-drag-region :title="project.name">{{ project.name }}</h1>
      <div class="detail-actions">
        <button v-if="status === 'complete'" class="form-button" :aria-pressed="editing"
                @click="editing = !editing"><Icon :name="editing ? 'check' : 'edit'" :size="16" />{{ editing ? '完成编辑' : '编辑对话' }}</button>
        <button v-if="status === 'complete'" class="form-button primary" :disabled="!hasCopyableText"
                @click="showExport"><Icon name="download" :size="16" />导出文件</button>
      </div>
    </header>

    <section class="player" :class="{ 'player-playing': playing }" aria-label="音频播放器">
      <audio v-if="audioUrl" ref="audio" :src="audioUrl" preload="metadata" @loadedmetadata="loadedAudio"
             @timeupdate="position = audio?.currentTime ?? 0" @play="playing = true" @pause="playing = false"
             @ended="playing = false" @error="audioError = '无法播放此音频，请检查原始文件是否有效。'" />
      <div class="player-controls">
        <button class="icon-button player-skip" title="后退 5 秒" aria-label="后退 5 秒"
                :disabled="!audioUrl || !duration || Boolean(audioError)" @click="seek(position - 5, true)"><Icon name="skip-back" :size="19" /></button>
        <button class="icon-button player-button" :aria-label="playing ? '暂停音频' : '播放音频'"
                :disabled="!audioUrl || Boolean(audioError)" @click="togglePlayback">
          <Icon :name="playing ? 'playing' : 'play'" :size="31" />
        </button>
        <button class="icon-button player-skip" title="前进 5 秒" aria-label="前进 5 秒"
                :disabled="!audioUrl || !duration || Boolean(audioError)" @click="seek(position + 5, true)"><Icon name="skip-next" :size="19" /></button>
      </div>
      <div class="timeline">
        <div class="play-times"><span>{{ clock(position) }}</span><span>{{ clock(duration) }}</span></div>
        <input type="range" aria-label="播放进度" min="0" :max="duration || 0" step="0.01" :value="position"
               :style="{ '--played': playbackProgress + '%' }" :disabled="!audioUrl || !duration || Boolean(audioError)"
               @input="seek(Number(($event.target as HTMLInputElement).value), true)" />
      </div>
      <select v-model.number="rate" class="rate" aria-label="播放速度" @change="changeRate">
        <option :value="0.75">0.75×</option>
        <option :value="1">1.0×</option>
        <option :value="1.25">1.25×</option>
        <option :value="1.5">1.5×</option>
        <option :value="2">2.0×</option>
      </select>
    </section>
    <p v-if="audioError || project.recordingError" class="detail-warning" role="status">{{ audioError || project.recordingError }}</p>

    <div v-if="status === 'complete'" class="transcript-toolbar">
      <div class="transcript-title">
        <h2>对话记录</h2>
        <span class="segment-count">{{ hasFilter ? `${shownSegments.length} / ` : '' }}{{ project.segments.length }} 段</span>
      </div>
      <div class="transcript-tools">
        <button v-if="editing || !project.segments.length" class="text-button" @click="addSegment">添加片段</button>
        <button v-if="project.segments.length" class="text-button" @click="showReplace">批量替换</button>
        <button v-if="hasCopyableText" class="text-button transcript-copy" @click="copyTranscript()"><Icon name="copy" :size="15" />复制全文</button>
        <label v-if="project.segments.length" class="search-box">
          <Icon name="search" /><input v-model="query" type="search" placeholder="搜索正文或姓名" aria-label="搜索对话记录" @keydown.enter="searchKey" />
        </label>
        <div v-if="query.trim() && shownSegments.length" class="search-navigation" role="group" aria-label="搜索结果定位">
          <span class="search-count" aria-live="polite">{{ searchMatchIndex < 0 ? `${shownSegments.length} 段` : `${searchMatchIndex + 1} / ${shownSegments.length}` }}</span>
          <button type="button" class="icon-button" title="上一个匹配项（Shift+Enter）" aria-label="上一个匹配项" @click="moveSearchMatch(-1)">
            <span aria-hidden="true">‹</span>
          </button>
          <button type="button" class="icon-button" title="下一个匹配项（Enter）" aria-label="下一个匹配项" @click="moveSearchMatch(1)">
            <span aria-hidden="true">›</span>
          </button>
        </div>
      </div>
    </div>
    <section v-if="status === 'complete'" class="speaker-panel" aria-label="说话人管理">
      <form v-if="renamingSpeakers" class="speaker-editor" @submit.prevent="saveSpeakerNames" @keydown.esc="renamingSpeakers = false">
        <label v-for="speaker in knownSpeakers" :key="speaker.id" :for="`speaker-name-${speaker.id}`">
          <span :class="`speaker-${speaker.id}`">说话人 {{ speaker.id }}</span>
          <input :id="`speaker-name-${speaker.id}`" v-model="speakerNames[speaker.id]" placeholder="输入姓名" maxlength="60" required />
        </label>
        <div class="speaker-edit-actions">
          <button type="button" class="form-button" @click="renamingSpeakers = false">取消</button>
          <button type="submit" class="form-button primary"
                  :disabled="knownSpeakers.some(speaker => !speakerNames[speaker.id]?.trim())">保存姓名</button>
        </div>
      </form>
      <div v-else class="speaker-bar">
        <div class="speaker-filters" aria-label="按说话人筛选">
          <button class="speaker-filter" :aria-pressed="speakerFilter === ''" @click="speakerFilter = ''">全部</button>
          <button v-for="speaker in knownSpeakers" :key="speaker.id" class="speaker-filter" :title="speaker.name"
                  :aria-pressed="speakerFilter === speaker.id" @click="speakerFilter = speaker.id">
            <span class="speaker-dot" :class="`speaker-${speaker.id}`"></span>
            <span class="filter-name">{{ speaker.name }}</span><span class="filter-count">{{ speakerCounts.get(speaker.id) ?? 0 }}</span>
          </button>
          <button v-if="speakerCounts.has('unknown')" class="speaker-filter" :aria-pressed="speakerFilter === 'unknown'"
                  @click="speakerFilter = 'unknown'">待确认 <span class="filter-count">{{ speakerCounts.get('unknown') }}</span></button>
        </div>
        <button class="text-button rename-speakers" @click="renameSpeakers()"><Icon name="edit" :size="15" />重命名说话人</button>
        <label v-if="project.segments.length" class="follow-playback"><input v-model="followPlayback" type="checkbox" />跟随播放</label>
      </div>
    </section>
    <div v-if="saveError" class="detail-warning" role="alert">{{ saveError }} <button class="text-button" @click="persist">重试保存</button></div>
    <p v-if="status === 'complete' && project.transcriptionError" class="detail-warning" role="status">{{ project.transcriptionError }}</p>
    <section v-if="status === 'complete' && dialogueTurns.length" class="transcript" aria-label="对话片段">
      <div v-for="turn in dialogueTurns" :key="turn.id" class="dialogue-turn" :class="speakerSide(turn.speakerId)">
        <span class="dialogue-avatar" :class="`speaker-${turn.speakerId}`" aria-hidden="true">{{ speakerInitial(turn.speakerId) }}</span>
        <div class="dialogue-bubble" role="group" :aria-labelledby="`turn-speaker-${turn.id}`">
          <div class="dialogue-caption">
            <span :id="`turn-speaker-${turn.id}`">{{ speakerName(turn.speakerId) }}</span>
            <button v-if="turn.speakerId !== 'unknown'" type="button" class="icon-button speaker-rename-button"
                    :title="`重命名 ${speakerName(turn.speakerId)}`" :aria-label="`重命名 ${speakerName(turn.speakerId)}`"
                    @click="renameSpeakers(turn.speakerId)"><Icon name="edit" :size="14" /></button>
          </div>
          <article v-for="segment in turn.segments" :id="`cue-${segment.id}`" :key="segment.id" class="segment"
                   :class="{ 'segment-current': segment.id === activeSegmentId, 'segment-search-current': segment.id === searchMatchId }"
                   :aria-current="segment.id === activeSegmentId ? 'true' : undefined">
            <div class="segment-meta">
              <span class="segment-assignment">
                <span aria-hidden="true">改归属 <span class="assignment-arrow">⌄</span></span>
                <select :value="segment.speakerId" :aria-label="`修改 ${clock(segment.start)} 的说话人`" @change="changeSpeaker(segment, $event)">
                  <option v-for="speaker in knownSpeakers" :key="speaker.id" :value="speaker.id">{{ speaker.name }}</option>
                  <option value="unknown">待确认</option>
                </select>
              </span>
              <span v-if="segment.id === activeSegmentId && playing" class="segment-playing"><Icon name="wave" :size="14" />播放中</span>
              <span v-if="segment.overlap" class="overlap-tag">重叠发言</span>
              <button class="timestamp" :aria-label="`跳转至 ${clock(segment.start)}`" @click="seek(segment.start)">{{ clock(segment.start) }}</button>
              <button v-if="segment.text.trim().length > 1 && (segment.end > segment.start || duration > segment.start)"
                      class="text-button split-action" @click="showSplit(segment)">拆分片段</button>
              <button v-if="segment.text.trim()" type="button" class="icon-button segment-copy-button" title="复制片段"
                      :aria-label="`复制 ${clock(segment.start)} 的片段`" @click="copyTranscript(segment)"><Icon name="copy" :size="15" /></button>
              <button v-if="editing && !segment.text.trim()" class="icon-button" aria-label="移除空片段" @click="removeBlankSegment(segment.id)"><Icon name="trash" :size="16" /></button>
            </div>
            <textarea v-if="editing || focused === segment.id" :id="`segment-${segment.id}`" v-model="segment.text" class="segment-edit" rows="1"
                      :aria-label="`编辑 ${clock(segment.start)} 的文字`" @input="persist" @blur="focused = ''"
                      @keydown.esc="($event.target as HTMLElement).blur()" />
            <button v-else class="segment-text" @click="beginEditing(segment, $event)">{{ segment.text || '点击编辑文字' }}</button>
          </article>
        </div>
      </div>
    </section>
    <div v-else class="transcript-empty">
      <template v-if="status === 'processing'">
        <div class="transcription-card recognition-progress">
          <span class="transcription-symbol processing-symbol" aria-hidden="true"><Icon name="wave" :size="27" /></span>
          <h2>正在转写录音</h2>
          <div class="recognition-label">
            <p role="status">{{ project.transcriptionStage || '正在转写' }}</p>
            <span v-if="project.transcriptionProgress != null">{{ project.transcriptionProgress }}%</span>
          </div>
          <div class="recognition-track" role="progressbar" aria-label="识别进度" aria-valuemin="0" aria-valuemax="100"
               :aria-valuenow="project.transcriptionProgress ?? undefined">
            <div class="recognition-fill" :class="{ 'recognition-pending': project.transcriptionProgress == null }"
                 :style="project.transcriptionProgress == null ? undefined : { width: `${project.transcriptionProgress}%` }"></div>
          </div>
        </div>
      </template>
      <template v-else-if="status === 'complete'">
        <div class="transcription-card">
          <span class="transcription-symbol" aria-hidden="true"><Icon :name="hasFilter ? 'search' : 'wave'" :size="24" /></span>
          <h2>{{ hasFilter ? '没有匹配的对话' : '没有转写片段' }}</h2>
          <button v-if="hasFilter" class="text-button" @click="query = ''; speakerFilter = ''">清除筛选</button>
          <p v-else class="field-help">可以播放音频，在当前时间添加片段。</p>
        </div>
      </template>
      <template v-else-if="project.recordingStatus === 'saved'">
        <div class="transcription-card">
          <span class="transcription-symbol" aria-hidden="true"><Icon name="wave" :size="27" /></span>
          <h2>{{ status === 'failed' ? '转写未完成' : '转写录音' }}</h2>
          <p v-if="project.transcriptionError" class="transcription-error" role="alert">{{ project.transcriptionError }}</p>
          <p v-else>音频已保存。</p>
          <p v-if="preferencesLoaded" class="method-summary">{{ methodLabels[method] }}<span v-if="method === 'api'"> · {{ onlineRouteDescription(apiModel) }}</span></p>
          <div class="transcribe-actions">
            <select v-model="language" class="form-button" aria-label="识别语言">
              <option value="auto">自动识别语言</option><option value="zh">中文</option><option value="en">英文</option>
            </select>
            <button class="pill" :disabled="busy || Boolean(audioError) || !preferencesLoaded" @click="startTranscription">
              <Icon name="wave" />{{ status === 'failed' ? '重新转写' : '开始转写' }}
            </button>
            <button class="text-button" :disabled="busy" @click="startManual">手动听写</button>
          </div>
        </div>
      </template>
    </div>

    <ElDialog v-model="splitDialog" title="拆分片段" width="min(520px, calc(100vw - 48px))" append-to-body>
      <form v-if="splitSegment" class="dialog-form" @submit.prevent="applySplit">
        <label for="split-text">在下方文字中点击两句发言的交界处</label>
        <textarea id="split-text" class="segment-edit split-text" :value="splitSegment.text" readonly rows="4"
                  @click="pickSplitPoint" @keyup="pickSplitPoint" @select="pickSplitPoint" />
        <template v-if="splitParts.every(Boolean)">
          <p class="split-preview">前半段 · {{ splitParts[0] }}</p>
          <p class="split-preview">后半段 · {{ splitParts[1] }}</p>
        </template>
        <label for="split-time">拆分时间（秒）</label>
        <input id="split-time" type="number" step="any" v-model.number="splitTime" required />
        <p class="field-help">初始时间按文字位置估算。可填入录音中实际换人的时间（{{ clock(splitSegment.start) }}–{{ clock(splitEnd) }}）。</p>
        <button v-if="position > splitSegment.start && position < splitEnd" type="button" class="text-button split-use-position"
                @click="splitTime = position">使用播放位置 {{ clock(position) }}</button>
        <div class="dialog-actions">
          <button type="button" class="form-button" @click="splitDialog = false">取消</button>
          <button type="submit" class="form-button primary" :disabled="!canSplit">确认拆分</button>
        </div>
      </form>
    </ElDialog>

    <ElDialog v-model="replaceDialog" title="批量替换" width="min(490px, calc(100vw - 48px))" append-to-body align-center>
      <form class="dialog-form" @submit.prevent="applyReplacement">
        <div class="settings-tabs" aria-label="替换方式">
          <button type="button" :aria-pressed="replaceMode === 'single'" @click="replaceMode = 'single'">查找并替换</button>
          <button type="button" :aria-pressed="replaceMode === 'global'" @click="replaceMode = 'global'">应用全局规则</button>
        </div>
        <template v-if="replaceMode === 'single'">
          <label for="find-text">查找文字</label>
          <input id="find-text" v-model="findText" required />
          <label for="replacement-text">替换为</label>
          <input id="replacement-text" v-model="replacementText" placeholder="留空则删除匹配文字" />
        </template>
        <p v-else-if="!globalRules.length" class="field-help">可在设置里添加全局替换规则。</p>
        <p class="field-help" aria-live="polite">当前记录有 {{ replacePreview.length }} 个片段将更新。</p>
        <div v-if="replacePreview.length" class="export-segments">
          <p v-for="change in replacePreview" :key="change.segment.id" class="replacement-preview">
            <span>{{ clock(change.segment.start) }}</span> {{ change.text || '（清空文字）' }}
          </p>
        </div>
        <div class="dialog-actions">
          <button type="button" class="form-button" @click="replaceDialog = false">取消</button>
          <button type="submit" class="form-button primary" :disabled="!replacePreview.length">全部替换</button>
        </div>
      </form>
    </ElDialog>

    <ElDialog v-model="exportDialog" title="导出转写" width="min(460px, calc(100vw - 48px))" append-to-body :close-on-click-modal="!busy">
      <form class="dialog-form" @submit.prevent="exportFile">
        <label for="export-format">文件格式</label>
        <select id="export-format" v-model="format"><option value="txt">TXT</option><option value="md">Markdown (.md)</option></select>
        <div class="export-options">
          <ElCheckbox v-model="timestamps">包含时间戳</ElCheckbox>
          <ElCheckbox v-model="speakerLabels">保留说话人标签</ElCheckbox>
          <ElCheckbox v-model="merge">合并相邻的同一说话人发言</ElCheckbox>
          <ElCheckbox v-model="selectExport">仅导出所选片段</ElCheckbox>
        </div>
        <div v-if="selectExport" class="export-segments">
          <label v-for="segment in project.segments" :key="segment.id"><input v-model="selected" type="checkbox" :value="segment.id" />
            <span>{{ clock(segment.start) }} {{ segment.text }}</span></label>
        </div>
        <div class="dialog-actions">
          <button type="button" class="form-button" :disabled="busy" @click="exportDialog = false">取消</button>
          <button type="submit" class="form-button primary" :disabled="busy || (selectExport && !selected.length)">选择位置并导出</button>
        </div>
      </form>
    </ElDialog>
  </main>
</template>
