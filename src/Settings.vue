<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { ElCheckbox } from 'element-plus'
import { nextTick, onMounted, ref } from 'vue'
import Icon from './Icon.vue'
import { call, errorMessage, methodLabels, onlineRouteDescription } from './desktop'
import type { Preferences } from './desktop'

interface ApiProfile {
  id: string
  name: string
  url: string
  model: string
  hasKey: boolean
}

interface ApiProfiles {
  profiles: ApiProfile[]
  activeId: string
  method: Preferences['method']
}

const emit = defineEmits<{ notice: [message: string, error?: boolean] }>()
const tab = ref<'general' | 'api' | 'rules'>('api')
const preferences = ref<Preferences>()
let persisted: Preferences | undefined
const api = ref<ApiProfiles>()
const busy = ref(false)
const draft = ref<ApiProfile>()
const nameField = ref<HTMLInputElement>()
const keyDraft = ref('')
const clearKey = ref(false)
const removing = ref('')

async function run(action: () => Promise<void>) {
  if (busy.value) return
  busy.value = true
  try { await action() } catch (error) { emit('notice', errorMessage(error), true) }
  finally { busy.value = false }
}

function load() {
  return run(async () => {
    const [prefs, profiles] = await Promise.all([call<Preferences>('preferences'), call<ApiProfiles>('api_profiles')])
    persisted = prefs
    preferences.value = { ...prefs, replacementRules: prefs.replacementRules.map(rule => ({ ...rule })) }
    api.value = profiles
  })
}
onMounted(load)

async function pickDirectory() {
  await run(async () => {
    const directory = await open({ title: '选择录音保存目录', directory: true })
    if (directory && preferences.value) preferences.value.audioDirectory = directory
  })
}

function uninstall() {
  return run(async () => { await call('open_uninstaller') })
}

function savePreferences() {
  const prefs = preferences.value
  if (!prefs || !persisted || !api.value) return
  const updates = tab.value === 'rules'
    ? { ...persisted, method: api.value.method, replacementRules: prefs.replacementRules.map(rule => ({ ...rule })) }
    : { ...prefs, replacementRules: persisted.replacementRules }
  return run(async () => {
    await call('save_preferences', { preferences: updates })
    persisted = updates
    api.value = await call<ApiProfiles>('api_profiles')
    emit('notice', '设置已保存')
  })
}

async function edit(profile?: ApiProfile, copy = false) {
  removing.value = ''
  draft.value = profile ? {
    ...profile,
    ...(copy ? { id: '', name: `${profile.name} 的副本`, hasKey: false } : {}),
  } : { id: '', name: '', url: '', model: '', hasKey: false }
  keyDraft.value = ''
  clearKey.value = false
  await nextTick()
  nameField.value?.focus()
}

function cancelEdit() {
  draft.value = undefined
  keyDraft.value = ''
  clearKey.value = false
}

function refreshProfiles(command: string, args: Record<string, unknown>, message: string) {
  return run(async () => {
    await call(command, args)
    cancelEdit()
    removing.value = ''
    api.value = await call<ApiProfiles>('api_profiles')
    if (preferences.value) preferences.value.method = api.value.method
    emit('notice', message)
  })
}

function saveProfile() {
  if (!draft.value) return
  return refreshProfiles('save_api_profile', {
    profile: { ...draft.value }, apiKey: keyDraft.value || null, clearKey: clearKey.value,
  }, '模型配置已保存')
}

function isActive(profile: ApiProfile) {
  return api.value?.method === 'api' && api.value.activeId === profile.id
}
</script>

<template>
  <main class="settings-page page-enter">
    <div class="settings-content">
      <div class="section-heading settings-heading"><h1>设置</h1></div>
      <nav class="settings-page-tabs" aria-label="设置分类">
        <button type="button" :aria-current="tab === 'general' ? 'page' : undefined" :disabled="busy" @click="tab = 'general'">通用</button>
        <button type="button" :aria-current="tab === 'api' ? 'page' : undefined" :disabled="busy" @click="tab = 'api'">AI 配置</button>
        <button type="button" :aria-current="tab === 'rules' ? 'page' : undefined" :disabled="busy" @click="tab = 'rules'">全局替换规则</button>
      </nav>
      <template v-if="preferences && api">
        <section v-if="tab === 'api'" class="api-section" aria-label="AI 模型配置">
          <div class="api-heading">
            <div>
              <h2>转写模型</h2>
              <p class="field-help">当前转写方式 · {{ methodLabels[api.method] }}</p>
            </div>
            <button type="button" class="form-button primary add-model" :disabled="busy || Boolean(draft)" @click="edit()">
              <Icon name="add" :size="16" />添加模型
            </button>
          </div>

          <form v-if="draft" class="dialog-form model-editor" @submit.prevent="saveProfile">
            <h3>{{ draft.id ? '编辑模型' : '添加模型' }}</h3>
            <div class="settings-grid">
              <label for="profile-name">配置名称
                <input id="profile-name" ref="nameField" v-model.trim="draft.name" placeholder="例如 百炼双人转写" maxlength="80" required />
              </label>
              <label for="profile-model">模型 ID
                <input id="profile-model" v-model.trim="draft.model" placeholder="例如 paraformer-v2" required />
              </label>
            </div>
            <label for="profile-url">服务 URL</label>
            <input id="profile-url" v-model.trim="draft.url" type="url" placeholder="https://服务地址/v1" required />
            <label for="profile-key">API Key</label>
            <input id="profile-key" v-model="keyDraft" type="password" autocomplete="off" :disabled="clearKey"
                   :placeholder="draft.hasKey ? '已保存，留空沿用' : '粘贴 API Key'" />
            <ElCheckbox v-if="draft.hasKey" v-model="clearKey" @change="keyDraft = ''">清除已保存的 Key</ElCheckbox>
            <p v-if="draft.model" class="field-help">{{ onlineRouteDescription(draft.model) }}</p>
            <div class="dialog-actions">
              <button type="button" class="form-button" :disabled="busy" @click="cancelEdit">取消</button>
              <button type="submit" class="form-button primary" :disabled="busy">保存模型</button>
            </div>
          </form>

          <p v-if="!api.profiles.length && !draft" class="api-empty">暂无在线模型。添加 URL、模型 ID 和 Key 后即可选用。</p>
          <article v-for="profile in api.profiles" :key="profile.id" class="model-card" :class="{ 'model-active': isActive(profile) }">
            <div class="model-card-row">
              <span class="model-icon"><Icon name="wave" :size="22" /></span>
              <div class="model-info">
                <h3 :title="profile.name">{{ profile.name }}</h3>
                <p :title="profile.model" class="model-id">{{ profile.model }}<span v-if="!profile.hasKey" class="missing-key"> · 缺少 Key</span></p>
                <p :title="profile.url" class="model-url">{{ profile.url }}</p>
              </div>
              <div class="model-actions">
                <button v-if="!isActive(profile)" type="button" class="form-button use-model" :aria-label="`使用 ${profile.name}`"
                        :disabled="busy || Boolean(draft) || !profile.hasKey"
                        @click="refreshProfiles('use_api_profile', { id: profile.id }, `已选用 ${profile.name}`)">使用</button>
                <span v-else class="model-in-use"><Icon name="check" :size="15" />使用中</span>
                <button type="button" class="icon-button" :title="`编辑 ${profile.name}`" :aria-label="`编辑 ${profile.name}`"
                        :disabled="busy || Boolean(draft)" @click="edit(profile)"><Icon name="edit" /></button>
                <button type="button" class="icon-button" :title="`复制 ${profile.name} 的模型和 URL`" :aria-label="`复制 ${profile.name}`"
                        :disabled="busy || Boolean(draft)" @click="edit(profile, true)"><Icon name="copy" :size="17" /></button>
                <button type="button" class="icon-button" :title="`删除 ${profile.name}`" :aria-label="`删除 ${profile.name}`"
                        :disabled="busy || Boolean(draft)" @click="removing = profile.id"><Icon name="trash" :size="17" /></button>
              </div>
            </div>
            <div v-if="removing === profile.id" class="model-delete">
              <p>{{ isActive(profile) ? '删除后切换到内置离线识别。' : `删除「${profile.name}」及其 API Key？` }}</p>
              <button type="button" class="form-button" :disabled="busy" @click="removing = ''">取消</button>
              <button type="button" class="form-button" :disabled="busy"
                      @click="refreshProfiles('delete_api_profile', { id: profile.id }, '已删除模型配置')">确认删除</button>
            </div>
          </article>
        </section>

        <div v-else-if="tab === 'general'" class="settings-form">
          <form class="dialog-form" @submit.prevent="savePreferences">
            <label for="directory">录音保存目录</label>
            <div class="field-row"><input id="directory" v-model="preferences.audioDirectory" required /><button type="button" class="form-button" :disabled="busy" @click="pickDirectory">选择</button></div>
            <div class="settings-grid">
              <label for="method">转写方式
                <select id="method" v-model="preferences.method">
                  <option v-for="(label, method) in methodLabels" :key="method" :value="method">{{ label }}</option>
                </select>
              </label>
              <label for="default-language">默认识别语言
                <select id="default-language" v-model="preferences.language">
                  <option value="auto">自动识别</option>
                  <option value="zh">中文</option>
                  <option value="en">英文</option>
                </select>
              </label>
            </div>
            <p v-if="preferences.method === 'api'" class="field-help">{{ api.activeId ? `在线模型 · ${api.profiles.find(profile => profile.id === api?.activeId)?.name}` : '请在 AI 配置中选择要使用的模型。' }}</p>
            <div class="dialog-actions"><button type="submit" class="form-button primary" :disabled="busy">保存设置</button></div>
          </form>
          <section class="uninstall-section" aria-labelledby="uninstall-title">
            <h2 id="uninstall-title">卸载声签</h2>
            <p class="field-help">打开 Windows 卸载向导时，声签会关闭。保留录音、转写记录和配置时，请让「删除应用数据」保持未勾选。</p>
            <button type="button" class="form-button uninstall-button" :disabled="busy" @click="uninstall">
              <Icon name="trash" :size="16" />打开卸载向导
            </button>
          </section>
        </div>

        <form v-else class="dialog-form settings-form" @submit.prevent="savePreferences">
          <p class="field-help">按列表顺序应用到新转写稿。替换文字留空时会删除匹配内容。</p>
          <div v-for="(rule, index) in preferences.replacementRules" :key="index" class="replacement-rule">
            <input v-model="rule.find" :aria-label="`规则 ${index + 1} 查找文字`" placeholder="查找文字" required />
            <input v-model="rule.replacement" :aria-label="`规则 ${index + 1} 替换为`" placeholder="替换为" />
            <button type="button" class="icon-button" :aria-label="`删除规则 ${index + 1}`" @click="preferences.replacementRules.splice(index, 1)"><Icon name="trash" :size="17" /></button>
          </div>
          <button type="button" class="form-button add-rule" @click="preferences.replacementRules.push({ find: '', replacement: '' })">添加规则</button>
          <div class="dialog-actions"><button type="submit" class="form-button primary" :disabled="busy">保存规则</button></div>
        </form>
      </template>
      <p v-else-if="busy" class="field-help" role="status">正在读取设置…</p>
      <button v-else type="button" class="form-button" @click="load">重新读取设置</button>
    </div>
  </main>
</template>
