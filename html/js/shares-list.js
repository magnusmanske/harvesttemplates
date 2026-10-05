// Publicly shared queries: searchable, sortable, with the outcome of their last run.
import { ref, computed, onMounted } from 'vue';
import { api } from './api.js';
import { user } from './session.js';
import { entityUrl, formatTime } from './links.js';

const COLUMNS = [
  { key: 'title', label: 'Title' },
  { key: 'wiki', label: 'Wiki' },
  { key: 'property', label: 'Property' },
  { key: 'template', label: 'Template' },
  { key: 'parameters', label: 'Parameter' },
  { key: 'user_name', label: 'Shared by' },
  { key: 'tagText', label: 'Tags' },
  { key: 'last_completed', label: 'Last complete run' },
];

/** Flatten a share into the sortable/searchable columns. */
function row(share) {
  const s = share.spec;
  const parameters = s.use_page_title ? '(page title)' : s.value_pattern ? s.value_pattern : s.date_parameters ? Object.values(s.date_parameters).filter(Boolean).join(' / ') : s.parameters.join(', ');
  return { ...share, wiki: `${s.siteid}.${s.project}`, property: s.property, template: s.template, parameters, tagText: share.tags.join(' ') };
}

export default {
  props: { tag: { type: String, default: '' } },
  setup(props) {
    const shares = ref(null);
    const error = ref('');
    const search = ref('');
    const sort = ref({ key: 'last_completed', descending: true });

    const visible = computed(() => {
      const needle = search.value.trim().toLowerCase();
      const { key, descending } = sort.value;
      return (shares.value ?? [])
        .filter((s) => !props.tag || s.tags.includes(props.tag))
        .filter((s) => !needle || COLUMNS.some((c) => String(s[c.key] ?? '').toLowerCase().includes(needle)))
        .sort((a, b) => (descending ? -1 : 1) * String(a[key] ?? '').localeCompare(String(b[key] ?? ''), undefined, { numeric: true }));
    });

    async function load() {
      try {
        shares.value = (await api('/shares')).map(row);
      } catch (e) {
        error.value = e.message;
      }
    }

    async function remove(share) {
      if (!confirm(`Delete "${share.title}"?`)) return;
      try {
        await api(`/shares/${share.id}`, { method: 'DELETE' });
        await load();
      } catch (e) {
        error.value = e.message;
      }
    }

    async function editTags(share) {
      const text = prompt('Tags, comma-separated', share.tags.join(', '));
      if (text === null) return;
      try {
        await api(`/shares/${share.id}/tags`, { method: 'PUT', body: { tags: text.split(',') } });
        await load();
      } catch (e) {
        error.value = e.message;
      }
    }

    const sortBy = (key) => { sort.value = { key, descending: sort.value.key === key ? !sort.value.descending : false }; };
    onMounted(load);
    return { shares, visible, error, search, sort, sortBy, remove, editTags, user, COLUMNS, entityUrl, formatTime, tagUrl: (t) => `#/shares/${encodeURIComponent(t)}` };
  },
  template: `
<div class="d-flex flex-wrap align-items-center gap-3 mb-2">
  <h1 class="h4 mb-0">Shared queries</h1>
  <input v-model="search" class="form-control ht-search" placeholder="Search" aria-label="search">
  <span v-if="tag" class="badge text-bg-primary fs-6">{{ tag }} <a href="#/shares" class="text-white ms-1" title="all tags">×</a></span>
</div>
<div v-if="error" class="alert alert-danger">{{ error }}</div>
<p v-else-if="!shares" class="text-muted">Loading…</p>
<p v-else-if="!shares.length" class="text-muted">Nothing shared yet. Use "Share publicly" on a run.</p>
<div v-else class="table-responsive">
  <table class="table table-sm table-hover">
    <thead><tr>
      <th v-for="c in COLUMNS" class="ht-sortable" @click="sortBy(c.key)">
        {{ c.label }} <span v-if="sort.key === c.key">{{ sort.descending ? '▼' : '▲' }}</span>
      </th>
      <th></th>
    </tr></thead>
    <tbody>
      <tr v-for="s in visible" :key="s.id">
        <td>{{ s.title }}</td>
        <td>{{ s.wiki }}</td>
        <td><a :href="entityUrl(s.property)" target="_blank" rel="noopener">{{ s.property }}</a></td>
        <td>{{ s.template }}</td>
        <td>{{ s.parameters }}</td>
        <td>{{ s.user_name }}</td>
        <td><a v-for="t in s.tags" :href="tagUrl(t)" class="badge text-bg-light text-decoration-none me-1">{{ t }}</a></td>
        <td>
          <template v-if="s.last_completed">
            {{ formatTime(s.last_completed) }}
            <span class="badge text-bg-success" title="added">{{ s.last_done }}</span>
            <span class="badge text-bg-danger" title="errors">{{ s.last_errors }}</span>
          </template>
        </td>
        <td class="text-nowrap">
          <a class="btn btn-sm btn-outline-primary" :href="'/?share=' + s.id">Open</a>
          <template v-if="user === s.user_name">
            <button class="btn btn-sm btn-outline-secondary ms-1" @click="editTags(s)">Tags</button>
            <button class="btn btn-sm btn-outline-danger ms-1" @click="remove(s)">Delete</button>
          </template>
        </td>
      </tr>
    </tbody>
  </table>
</div>`,
};
