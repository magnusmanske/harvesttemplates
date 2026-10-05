// One run: progress, actions, and the per-page results.
import { ref, computed, watch, onMounted, onUnmounted } from 'vue';
import { api } from './api.js';
import { user, login } from './session.js';
import { pageUrl, entityUrl, editGroupUrl, formatTime, statusColor, rowStatusLabel, valueUrl } from './links.js';

const PAGE_SIZE = 100;
const POLL_MS = 2000;
const FILTERS = ['', 'pending', 'ready', 'done', 'skipped', 'error'];
const EXCLUDED_LABELS = {
  not_in_category: 'not in the category',
  not_in_list: 'not in the list',
  not_in_petscan: 'not in the PetScan result',
  not_in_sparql: 'not in the SPARQL result',
  no_item: 'without Wikidata item',
  not_instance: 'not instances of the classes',
  already_set: 'already have the property',
};

export default {
  props: { id: { type: Number, required: true } },
  setup(props) {
    const info = ref(null);
    const rows = ref([]);
    const property = ref(null);
    const filter = ref('');
    const offset = ref(0);
    const error = ref('');
    let timer = null;

    const run = computed(() => info.value?.run);
    const counts = computed(() => info.value?.counts ?? {});
    const total = computed(() => Object.values(counts.value).reduce((a, b) => a + b, 0));
    const busy = computed(() => info.value?.active || run.value?.status === 'loading');
    const isOwner = computed(() => user.value && run.value && user.value === run.value.user_name);
    const canWork = computed(() => isOwner.value && !busy.value && counts.value.pending + counts.value.ready > 0);
    const progress = computed(() => {
      const c = counts.value;
      return total.value ? Math.round((100 * (c.done + c.skipped + c.error + c.ready)) / total.value) : 0;
    });
    const excluded = computed(() => Object.entries(run.value?.excluded ?? {})
      .filter(([, n]) => n > 0).map(([key, n]) => `${n.toLocaleString()} ${EXCLUDED_LABELS[key] ?? key}`));

    async function refresh() {
      clearTimeout(timer);
      try {
        info.value = await api(`/runs/${props.id}`);
        if (!property.value) property.value = await api(`/property/${info.value.run.spec.property}`).catch(() => null);
        const status = filter.value ? `&status=${filter.value}` : '';
        rows.value = await api(`/runs/${props.id}/rows?offset=${offset.value}&limit=${PAGE_SIZE}${status}`);
        error.value = '';
      } catch (e) {
        error.value = e.message;
      }
      document.title = `${busy.value ? '' : 'DONE - '}Harvest Templates run ${props.id}`;
      if (busy.value) timer = setTimeout(refresh, POLL_MS);
    }

    async function act(action, confirmText) {
      if (!user.value) return login();
      if (confirmText && !confirm(confirmText)) return;
      try {
        await api(`/runs/${props.id}/${action}`, { method: 'POST' });
        setTimeout(refresh, 500);
      } catch (e) {
        error.value = e.message;
      }
    }

    async function share() {
      const title = prompt('Title for the shared query, e.g. "IMDb IDs from enwiki films"');
      if (!title) return;
      const tags = (prompt('Tags, comma-separated (optional), e.g. "enwiki, films"') ?? '').split(',');
      try {
        await api('/shares', { method: 'POST', body: { title, spec: run.value.spec, tags } });
        location.hash = '#/shares';
      } catch (e) {
        error.value = e.message;
      }
    }

    watch(filter, () => { offset.value = 0; refresh(); });
    watch(offset, refresh);
    watch(() => props.id, refresh);
    onMounted(refresh);
    onUnmounted(() => { clearTimeout(timer); document.title = 'Harvest Templates'; });

    return {
      info, run, counts, total, rows, filter, offset, error, busy, isOwner, canWork, progress, excluded, FILTERS, PAGE_SIZE,
      refresh, share, login, pageUrl, entityUrl, editGroupUrl, formatTime, statusColor, rowStatusLabel, property, valueUrl,
      preview: () => act('preview'),
      start: () => act('start', `Add up to ${(counts.value.pending + counts.value.ready).toLocaleString()} statements to Wikidata as ${user.value}?`),
      stop: () => act('stop'),
    };
  },
  template: `
<div v-if="!info && !error" class="text-muted p-4">Loading…</div>
<div v-else-if="!info" class="alert alert-danger">{{ error }}</div>
<div v-else>
  <div class="d-flex flex-wrap align-items-baseline gap-2 mb-2">
    <h1 class="h4 mb-0">Run {{ run.id }}</h1>
    <span class="text-muted">{{ run.spec.property }} from {{ run.spec.template }} on {{ info.host }}</span>
    <span class="badge" :class="'text-bg-' + statusColor(run.status)">{{ run.status }}</span>
    <span class="text-muted small ms-auto">by {{ run.user_name }}, {{ formatTime(run.created) }}</span>
  </div>
  <div v-if="run.message" class="alert py-2" :class="run.status === 'failed' ? 'alert-danger' : 'alert-secondary'">{{ run.message }}</div>
  <div v-if="error" class="alert alert-danger py-2">{{ error }}</div>

  <div v-if="run.status === 'loading'" class="alert alert-warning">Collecting pages… large templates and deep categories can take a minute.</div>
  <template v-else>
    <div class="progress mb-2" role="progressbar" :aria-valuenow="progress" aria-valuemin="0" aria-valuemax="100" style="height: 1.25rem">
      <div class="progress-bar bg-success" :style="{ width: (100 * counts.done / (total || 1)) + '%' }">{{ counts.done || '' }}</div>
      <div class="progress-bar bg-info" :style="{ width: (100 * counts.ready / (total || 1)) + '%' }">{{ counts.ready || '' }}</div>
      <div class="progress-bar bg-secondary" :style="{ width: (100 * counts.skipped / (total || 1)) + '%' }">{{ counts.skipped || '' }}</div>
      <div class="progress-bar bg-danger" :style="{ width: (100 * counts.error / (total || 1)) + '%' }">{{ counts.error || '' }}</div>
    </div>
    <p class="small text-muted mb-2">
      {{ total.toLocaleString() }} candidate pages<span v-if="excluded.length">; left out: {{ excluded.join(', ') }}</span>.
    </p>
  </template>

  <div class="d-flex flex-wrap gap-2 mb-3">
    <template v-if="isOwner">
      <button class="btn btn-outline-primary" :disabled="!canWork || counts.pending === 0" @click="preview" title="check every page without editing">Preview</button>
      <button class="btn btn-primary" :disabled="!canWork" @click="start">Start editing</button>
      <button class="btn btn-outline-danger" :disabled="!info.active" @click="stop">Stop</button>
    </template>
    <button v-else-if="!user" class="btn btn-outline-primary" @click="login">Log in</button>
    <a class="btn btn-outline-secondary" :href="'/api/runs/' + run.id + '/log.csv'">Download log</a>
    <a class="btn btn-outline-secondary" :href="'/?' + info.permalink">Edit as new run</a>
    <button class="btn btn-outline-secondary" @click="share">Share publicly</button>
    <a v-if="counts.done" class="btn btn-outline-secondary" :href="editGroupUrl(run.editgroup)" target="_blank" rel="noopener">Edit group</a>
  </div>

  <ul class="nav nav-tabs">
    <li v-for="f in FILTERS" class="nav-item">
      <a class="nav-link" :class="{ active: filter === f }" href="" @click.prevent="filter = f">
        {{ f ? rowStatusLabel(f) : 'all' }} <span class="badge text-bg-light">{{ (f ? counts[f] : total) ?? 0 }}</span>
      </a>
    </li>
  </ul>
  <div class="table-responsive">
    <table class="table table-sm ht-rows mb-1">
      <thead><tr><th>Page</th><th>Item</th><th>Template value</th><th>Value</th><th>Result</th></tr></thead>
      <tbody>
        <tr v-for="r in rows" :key="r.seq" :class="'ht-row-' + r.status">
          <td><a :href="pageUrl(info.host, r.title)" target="_blank" rel="noopener">{{ r.title }}</a></td>
          <td><a v-if="r.item" :href="entityUrl(r.item)" target="_blank" rel="noopener">{{ r.item }}</a></td>
          <td class="ht-value">{{ r.raw_value }}</td>
          <td class="ht-value">
            <a v-if="valueUrl(property, r.value)" :href="valueUrl(property, r.value)" target="_blank" rel="noopener">{{ r.value }}</a>
            <template v-else>{{ r.value }}</template>
          </td>
          <td><span class="badge" :class="'text-bg-' + statusColor(r.status)">{{ rowStatusLabel(r.status) }}</span> {{ r.message }}</td>
        </tr>
        <tr v-if="!rows.length"><td colspan="5" class="text-muted">No pages.</td></tr>
      </tbody>
    </table>
  </div>
  <nav class="d-flex gap-2 align-items-center">
    <button class="btn btn-sm btn-outline-secondary" :disabled="offset === 0" @click="offset -= PAGE_SIZE">‹ previous</button>
    <span class="small text-muted">{{ offset + 1 }}–{{ offset + rows.length }}</span>
    <button class="btn btn-sm btn-outline-secondary" :disabled="rows.length < PAGE_SIZE" @click="offset += PAGE_SIZE">next ›</button>
  </nav>
</div>`,
};
