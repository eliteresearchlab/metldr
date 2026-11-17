<script setup lang="ts">
import { Binary } from 'lucide-vue-next'
import { computed } from 'vue'
import { ROUTE_NAMES, type GatewayRoute } from '@/composables/useGateway'

interface Props {
  routes: GatewayRoute[]
  active: GatewayRoute | null
  detecting: boolean
}

const props = defineProps<Props>()

const emit = defineEmits<{
  prefer: [route: string, model: string]
  'open-welcome': []
}>()

const STATUS: Record<GatewayRoute['state'], string> = {
  ready: 'ready',
  downloadable: 'download',
  downloading: 'downloading',
  absent: 'offline',
  'needs-key': 'needs key',
  error: 'error'
}

// the routes that can answer now or soon, and the chosen one even when it is offline
const shown = computed(() => props.routes.filter(r =>
  ['ready', 'downloadable', 'downloading'].includes(r.state) || r.route === props.active?.route))

const models = computed(() => props.active?.models ?? [])

function choose(route: GatewayRoute): void {
  emit('prefer', route.route, route.model ?? route.models?.[0] ?? '')
}
</script>

<template>
  <div class="rounded-xl bg-card p-4 border border-border space-y-3">
    <div class="flex items-center gap-2.5">
      <div class="flex items-center justify-center w-6 h-6 rounded-md bg-primary/25 shrink-0">
        <Binary :size="12" class="text-primary" />
      </div>
      <span class="text-(length:--font-text-secondary) font-medium text-foreground tracking-wide">ai provider</span>
    </div>

    <button
      v-for="route in shown"
      :key="route.route"
      @click="choose(route)"
      :aria-pressed="route.route === active?.route"
      class="w-full flex items-center justify-between gap-2 py-2 px-3 rounded-lg transition-colors active:scale-[0.97]"
      :class="route.route === active?.route ? 'bg-primary/10 ring-1 ring-primary/30' : 'bg-muted/30 hover:bg-muted/50'"
    >
      <span class="flex flex-col items-start min-w-0 text-start">
        <span class="text-(length:--font-text-secondary) font-medium" :class="route.route === active?.route ? 'text-primary' : 'text-foreground/70'">
          {{ ROUTE_NAMES[route.route] ?? route.route }}
        </span>
        <span class="text-(length:--font-text-secondary) text-foreground/50 truncate max-w-full" :title="route.destination">{{ route.destination }}</span>
      </span>
      <span
        class="text-(length:--font-text-secondary) px-1.5 py-0.5 rounded-full font-medium shrink-0 whitespace-nowrap"
        :class="{
          'bg-emerald-500/20 text-emerald-400': route.state === 'ready',
          'bg-blue-500/20 text-blue-400': route.state === 'downloadable',
          'bg-amber-500/20 text-amber-400': route.state === 'downloading',
          'bg-red-500/20 text-red-400': route.state === 'absent' || route.state === 'error' || route.state === 'needs-key'
        }"
      >{{ STATUS[route.state] }}</span>
    </button>

    <select
      v-if="active && models.length > 1"
      :value="active.model"
      @change="emit('prefer', active.route, ($event.target as HTMLSelectElement).value)"
      aria-label="model"
      class="w-full px-3 py-2 bg-muted/50 rounded-lg border border-input font-mono text-(length:--font-text-secondary) text-foreground"
    >
      <option v-for="model in models" :key="model" :value="model">{{ model }}</option>
    </select>

    <p class="text-(length:--font-text-secondary) text-foreground/50 pt-1">
      <template v-if="detecting">looking for ai on this computer...</template>
      <template v-else-if="!shown.length">no ai found yet.</template>
      <template v-else-if="active?.state === 'downloadable'">download the model in setup.</template>
      <button @click="emit('open-welcome')" class="text-primary hover:underline ms-1">
        {{ !shown.length ? 'set up ai' : active?.state === 'downloadable' ? 'open setup' : 'add another' }}
      </button>
    </p>
  </div>
</template>
