<script setup lang="ts" generic="Value extends string | number">
import { ElOption, ElSelect } from 'element-plus'
import type { SelectInstance } from 'element-plus'
import { ref, watchPostEffect } from 'vue'

const props = defineProps<{
  id?: string
  ariaLabel?: string
  ariaDescribedby?: string
  options: { value: Value; label: string }[]
  displayLabel?: string
}>()
const value = defineModel<Value>({ required: true })
const select = ref<SelectInstance>()

watchPostEffect(() => {
  const input = select.value?.inputRef
  if (!input) return
  if (props.ariaDescribedby) input.setAttribute('aria-describedby', props.ariaDescribedby)
  else input.removeAttribute('aria-describedby')
})

defineExpose({ focus: () => select.value?.focus() })
</script>

<template>
  <ElSelect :id="id" ref="select" v-model="value" :aria-label="ariaLabel" class="voice-select"
            popper-class="voice-select-popup" :empty-values="[null, undefined]"
            :show-arrow="false" :offset="4" :persistent="false" placement="bottom-start">
    <template v-if="displayLabel" #label>{{ displayLabel }}</template>
    <ElOption v-for="option in options" :key="option.value" :label="option.label" :value="option.value" />
  </ElSelect>
</template>
