// A list of properties with their values: qualifiers for every harvested statement
// (a fixed value, or another parameter of the template), or, with `extra`, more
// properties to harvest from the same template (#111).
import { reactive, watch } from 'vue';
import { api } from './api.js';

const PLACEHOLDERS = { 'wikibase-item': 'Q1860', time: '2024-05-01', quantity: '42', monolingualtext: 'text@en' };
const splitNames = (text) => text.split(',').map((n) => n.trim()).filter(Boolean);

export default {
  props: { modelValue: { type: Array, required: true }, extra: { type: Boolean, default: false } },
  emits: ['update:modelValue'],
  setup(props, { emit }) {
    const rows = reactive(props.modelValue.map((q) => ({
      property: q.property, source: q.source ?? 'parameter', value: q.value ?? '', names: (q.names ?? q.parameters ?? []).join(', '), info: null,
    })));
    const lookup = async (row) => {
      row.info = null;
      if (/^P\d+$/.test(row.property)) row.info = await api(`/property/${row.property}`).catch((e) => ({ error: e.message }));
    };
    rows.forEach(lookup);

    // Only complete rows reach the spec.
    watch(rows, () => emit('update:modelValue', rows
      .filter((r) => /^P\d+$/.test(r.property) && (r.source === 'fixed' ? r.value.trim() : splitNames(r.names).length))
      .map((r) => {
        if (props.extra) return { property: r.property, parameters: splitNames(r.names) };
        return r.source === 'fixed'
          ? { property: r.property, source: 'fixed', value: r.value.trim() }
          : { property: r.property, source: 'parameter', names: splitNames(r.names) };
      })), { deep: true });

    return {
      rows,
      add: () => rows.push({ property: '', source: props.extra ? 'parameter' : 'fixed', value: '', names: '', info: null }),
      remove: (i) => rows.splice(i, 1),
      setProperty: (row, text) => {
        const m = text.trim().match(/^[Pp]?(\d+)$/);
        row.property = m ? `P${m[1]}` : text.trim();
        lookup(row);
      },
      placeholder: (row) => (row.source === 'fixed' ? PLACEHOLDERS[row.info?.datatype] ?? 'value' : 'parameter, aliases'),
    };
  },
  template: `
<div v-for="(row, i) in rows" class="mb-2">
  <div class="input-group input-group-sm">
    <input class="form-control ht-short" :value="row.property" @change="setProperty(row, $event.target.value)" placeholder="P407" aria-label="qualifier property">
    <select v-if="!extra" v-model="row.source" class="form-select ht-short" aria-label="source">
      <option value="fixed">value</option>
      <option value="parameter">param</option>
    </select>
    <input v-if="row.source === 'fixed'" v-model="row.value" class="form-control" :placeholder="placeholder(row)">
    <input v-else v-model="row.names" class="form-control" :placeholder="extra ? 'parameter, aliases' : placeholder(row)">
    <button type="button" class="btn btn-outline-secondary" @click="remove(i)" title="remove">×</button>
  </div>
  <div class="form-text">
    <span v-if="row.info?.error" class="text-danger">{{ row.info.error }}</span>
    <span v-else-if="row.info">{{ row.info.label }} · {{ row.info.datatype }}<span v-if="!row.info.supported" class="text-danger"> · not supported</span></span>
  </div>
</div>
<button type="button" class="btn btn-sm btn-link px-0" @click="add">+ add {{ extra ? 'property' : 'qualifier' }}</button>`,
};
