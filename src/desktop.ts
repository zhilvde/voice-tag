import { invoke } from '@tauri-apps/api/core'

export interface Speaker {
  id: string
  name: string
}

export interface Segment {
  id: string
  start: number
  end: number
  text: string
  speakerId: string
  overlap: boolean
}

export interface Project {
  id: string
  name: string
  createdAt: string
  duration: number
  source: 'microphone' | 'import'
  recordingStatus: 'recording' | 'saved' | 'failed'
  recordingError: string | null
  transcriptionStatus: 'pending' | 'processing' | 'complete' | 'failed'
  transcriptionStage: string
  transcriptionProgress: number | null
  transcriptionError: string | null
  language: 'auto' | 'zh' | 'en'
  speakers: Speaker[]
  segments: Segment[]
}

export interface Preferences {
  audioDirectory: string
  language: Project['language']
  method: 'windows' | 'api' | 'local'
  apiUrl: string
  apiModel: string
  hasApiKey: boolean
  replacementRules: ReplacementRule[]
}

export interface ReplacementRule {
  find: string
  replacement: string
}

export const methodLabels: Record<Preferences['method'], string> = {
  windows: 'Windows 离线识别',
  api: '在线 API',
  local: '内置离线识别',
}

export function onlineRouteDescription(model: string) {
  const name = model.trim().toLowerCase().split('/').at(-1) ?? ''
  if (name.startsWith('deepseek-')) return '本机先转写，再把识别文字发送给 DeepSeek 校对。音频留在本机。'
  if (name === 'paraformer-v2') return '录音会上传到百炼私有临时存储，并自动区分说话人。'
  if (name.startsWith('qwen3-asr-flash-filetrans')) return '百炼返回逐句时间戳。需要自动区分说话人时，可切换到 paraformer-v2。'
  if (name.startsWith('qwen3-asr-flash')) return '使用 Qwen-ASR 转写。需要自动区分说话人时，可切换到 paraformer-v2。'
  return '音频会发送到配置的服务，使用语音转写接口转写。'
}

export interface RecordingStatus {
  paused: boolean
  duration: number
  level: number
  error: string | null
}

export const call = invoke

export function clock(seconds: number, hours = false) {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.floor(seconds)) : 0
  const parts = [Math.floor(total / 60) % 60, total % 60]
  if (hours || total >= 3600) parts.unshift(Math.floor(total / 3600))
  return parts.map(part => String(part).padStart(2, '0')).join(':')
}

export function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error)
}
