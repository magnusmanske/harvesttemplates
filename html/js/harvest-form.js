// The harvest form: builds a JobSpec, then creates a run.
import { reactive, ref, computed, watch, onMounted } from 'vue';
import { api, toQuery } from './api.js';
import { user, login } from './session.js';
import { entityUrl } from './links.js';
import QualifierEditor from './qualifier-editor.js';

const PROJECTS = ['wikipedia', 'wikibooks', 'wikinews', 'wikiquote', 'wikisource', 'wikiversity', 'wikivoyage', 'wiktionary', 'commons', 'species'];

/** Run `fn` once `delay` ms after the last call. */
function debounced(fn, delay = 400) {
  let timer;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), delay);
  };
}

/** `1582-10-15`, `1582-10` or `1582` ⇄ {year, month, day}. */
const dateText = (d) => (d ? [d.year, d.month, d.day].filter((n) => n > 0).join('-') : '');
function parseDateText(text) {
  const m = text.trim().match(/^(\d{1,4})(?:-(\d{1,2}))?(?:-(\d{1,2}))?$/);
  return m ? { year: Number(m[1]), month: Number(m[2] ?? 0), day: Number(m[3] ?? 0) } : null;
}

/** Toggle `name` in an optional selection where `null` means "all of `all`". */
function toggled(selection, all, name) {
  const current = new Set(selection ?? all);
  current.has(name) ? current.delete(name) : current.add(name);
  return all.every((n) => current.has(n)) ? null : all.filter((n) => current.has(n));
}

export default {
  components: { QualifierEditor },
  setup() {
    const spec = reactive({});
    const ready = ref(false);
    const shareId = ref(null);
    const notice = ref('');
    const error = ref('');
    const busy = ref(false);
    const site = ref(null);
    const template = ref(null);
    const property = ref(null);
    const lookupErrors = reactive({ site: '', template: '', property: '' });
    const permalink = ref('/');
    const limitText = ref('');
    const propertyText = ref('');

    const datatype = computed(() => property.value?.datatype);
    const namespaces = computed(() => Object.entries(site.value?.namespaces ?? { 0: '' })
      .map(([id, name]) => ({ id: Number(id), name: name || '(main)' }))
      .filter((ns) => ns.id >= 0));
    const redirects = computed(() => template.value?.redirects ?? []);
    const checkable = computed(() => (property.value?.constraints ?? []).filter((c) => c.supported && c.status === 'normal').map((c) => c.id));
    const dateMode = computed({
      get: () => (spec.date_parameters ? 'parts' : 'single'),
      set: (mode) => { spec.date_parameters = mode === 'parts' ? { year: '', month: '', day: '' } : null; },
    });
    const coordinateMode = computed({
      get: () => (spec.coordinate_parameters ? 'parts' : 'single'),
      set: (mode) => { spec.coordinate_parameters = mode === 'parts' ? { latitude: '', longitude: '' } : null; },
    });

    const lookup = async (key, target, request) => {
      try {
        target.value = await request();
        lookupErrors[key] = '';
      } catch (e) {
        target.value = null;
        lookupErrors[key] = e.message;
      }
    };
    const loadSite = debounced(() => lookup('site', site, () => api(`/site?${toQuery({ siteid: spec.siteid, project: spec.project })}`)));
    const loadTemplate = debounced(() => {
      if (!spec.template.trim()) return (template.value = null);
      lookup('template', template, () => api(`/template?${toQuery({ siteid: spec.siteid, project: spec.project, template: spec.template })}`));
    });
    const loadProperty = debounced(async () => {
      if (!spec.property) return (property.value = null);
      await lookup('property', property, () => api(`/property/${spec.property}`));
      const units = property.value?.units;
      if (units?.length && !units.some((u) => u.id === spec.unit)) spec.unit = units[0].id;
    });
    const updatePermalink = debounced(async () => {
      try {
        permalink.value = `/?${(await api('/spec/to-query', { method: 'POST', body: spec })).query}`;
      } catch { /* keep the previous link */ }
    }, 500);

    watch(() => [spec.siteid, spec.project], () => { loadSite(); loadTemplate(); });
    watch(() => spec.template, loadTemplate);
    watch(() => spec.property, loadProperty);
    watch(propertyText, (text) => {
      const m = text.trim().match(/^[Pp]?(\d+)$/);
      spec.property = m ? `P${m[1]}` : null;
    });
    watch(limitText, (text) => {
      if (!spec.date_limit) return;
      const date = parseDateText(text);
      if (date) spec.date_limit.date = date;
    });
    watch(spec, updatePermalink, { deep: true });

    async function initialise() {
      const params = new URLSearchParams(location.search);
      Object.assign(spec, await api(`/spec/from-query?${params}`));
      // ?share=ID, or ?htid=ID for queries shared in the old tool (which loaded them right away).
      const shareUrl = params.has('share') ? `/shares/${params.get('share')}` : params.has('htid') ? `/shares/legacy/${params.get('htid')}` : null;
      if (shareUrl) {
        try {
          const share = await api(shareUrl);
          Object.assign(spec, share.spec);
          shareId.value = share.id;
          notice.value = `Loaded the shared query "${share.title}" by ${share.user_name}.`;
        } catch {
          notice.value = 'This shared query does not exist (any more).';
        }
      }
      if (!spec.parameters.length) spec.parameters.push('');
      propertyText.value = spec.property ?? '';
      limitText.value = dateText(spec.date_limit?.date);
      ready.value = true;
      if ((params.has('run') || (params.has('htid') && shareId.value)) && user.value) await submit();
    }

    async function submit() {
      if (!user.value) return login();
      busy.value = true;
      error.value = '';
      try {
        const { id } = await api('/runs', { method: 'POST', body: { spec, share_id: shareId.value } });
        location.hash = `#/run/${id}`;
      } catch (e) {
        error.value = e.message;
      } finally {
        busy.value = false;
      }
    }

    const setLimit = (on) => {
      spec.date_limit = on ? { relation: 'at_least', date: parseDateText(limitText.value || '1926') } : null;
      if (on && !limitText.value) limitText.value = '1926';
    };
    const instanceOfText = computed({
      get: () => spec.instance_of?.join(', ') ?? '',
      set: (text) => { spec.instance_of = text.split(/[\s,|]+/).map((q) => q.trim().toUpperCase()).filter((q) => /^Q\d+$/.test(q)); },
    });
    const manualListText = computed({
      get: () => spec.manual_list?.join('\n') ?? '',
      set: (text) => { spec.manual_list = text.split('\n').map((l) => l.trim()).filter(Boolean); },
    });

    onMounted(() => initialise().catch((e) => { error.value = e.message; ready.value = true; }));

    return {
      spec, ready, notice, error, busy, site, template, property, lookupErrors, permalink, limitText, propertyText,
      datatype, namespaces, redirects, checkable, dateMode, coordinateMode, manualListText, instanceOfText, PROJECTS, user,
      submit, setLimit, entityUrl,
      redirectChecked: (name) => spec.template_redirects === null || spec.template_redirects.includes(name),
      toggleRedirect: (name) => { spec.template_redirects = toggled(spec.template_redirects, redirects.value, name); },
      constraintChecked: (c) => c.status === 'mandatory' || (c.supported && c.status === 'normal' && (spec.constraints === null || spec.constraints.includes(c.id))),
      toggleConstraint: (id) => { spec.constraints = toggled(spec.constraints, checkable.value, id); },
    };
  },
  template: `
<div v-if="!ready" class="text-muted p-4">Loading…</div>
<form v-else @submit.prevent="submit" class="ht-form">
  <div v-if="notice" class="alert alert-info py-2">{{ notice }}</div>
  <div class="row g-3">
    <section class="col-xl-3 col-md-6">
      <div class="card h-100"><div class="card-body">
        <h2 class="h5">Load pages from</h2>
        <label class="form-label">Wiki</label>
        <div class="input-group mb-1">
          <input v-model.trim="spec.siteid" class="form-control ht-short" :class="{ 'is-invalid': lookupErrors.site }" placeholder="en" aria-label="language code"
                 :disabled="['commons', 'species'].includes(spec.project)">
          <span class="input-group-text">.</span>
          <select v-model="spec.project" class="form-select" aria-label="project">
            <option v-for="p in PROJECTS" :value="p">{{ p }}</option>
          </select>
        </div>
        <div class="form-text text-danger mb-2" v-if="lookupErrors.site">{{ lookupErrors.site }}</div>
        <label class="form-label mt-2">Namespace</label>
        <select v-model.number="spec.namespace" class="form-select">
          <option v-for="ns in namespaces" :value="ns.id">{{ ns.name }}</option>
        </select>

        <h2 class="h5 mt-4">Define import</h2>
        <label class="form-label">Property</label>
        <input v-model="propertyText" class="form-control" :class="{ 'is-invalid': lookupErrors.property || property?.deprecated || (property && !property.supported) }" placeholder="P345">
        <div class="form-text">
          <span v-if="property">
            <a :href="entityUrl(property.id)" target="_blank" rel="noopener">{{ property.label }}</a> · {{ property.datatype }}
            <span v-if="!property.supported" class="text-danger"> · not supported</span>
            <span v-if="property.deprecated" class="text-danger"> · deprecated</span>
          </span>
          <span v-else class="text-danger">{{ lookupErrors.property }}</span>
        </div>
        <label class="form-label mt-2">Template</label>
        <input v-model.trim="spec.template" class="form-control" :class="{ 'is-invalid': template && !template.exists }" placeholder="Infobox person">
        <div class="form-text">
          <a v-if="template?.exists" :href="template.url" target="_blank" rel="noopener">{{ template.name }}</a>
          <span v-else-if="template" class="text-danger">does not exist</span>
        </div>
        <div v-if="redirects.length" class="mt-2">
          <div class="form-label mb-1">Also accept the redirects</div>
          <div class="ht-scroll">
            <div v-for="r in redirects" class="form-check">
              <input type="checkbox" class="form-check-input" :id="'r-' + r" :checked="redirectChecked(r)" @change="toggleRedirect(r)">
              <label class="form-check-label" :for="'r-' + r">{{ r }}</label>
            </div>
          </div>
        </div>
      </div></div>
    </section>

    <section class="col-xl-3 col-md-6">
      <div class="card h-100"><div class="card-body">
        <h2 class="h5">Value</h2>
        <div v-if="datatype === 'time'" class="btn-group btn-group-sm mb-2" role="group">
          <input type="radio" class="btn-check" id="dm-single" value="single" v-model="dateMode"><label class="btn btn-outline-secondary" for="dm-single">one parameter</label>
          <input type="radio" class="btn-check" id="dm-parts" value="parts" v-model="dateMode"><label class="btn btn-outline-secondary" for="dm-parts">year / month / day</label>
        </div>
        <div v-if="datatype === 'globe-coordinate'" class="btn-group btn-group-sm mb-2" role="group">
          <input type="radio" class="btn-check" id="cm-single" value="single" v-model="coordinateMode"><label class="btn btn-outline-secondary" for="cm-single">one parameter</label>
          <input type="radio" class="btn-check" id="cm-parts" value="parts" v-model="coordinateMode"><label class="btn btn-outline-secondary" for="cm-parts">latitude / longitude</label>
        </div>
        <template v-if="spec.coordinate_parameters">
          <input v-model.trim="spec.coordinate_parameters.latitude" class="form-control mb-1" placeholder="latitude parameter">
          <input v-model.trim="spec.coordinate_parameters.longitude" class="form-control mb-1" placeholder="longitude parameter">
        </template>
        <template v-else-if="spec.date_parameters">
          <input v-model.trim="spec.date_parameters.year" class="form-control mb-1" placeholder="year parameter">
          <input v-model.trim="spec.date_parameters.month" class="form-control mb-1" placeholder="month parameter (optional)">
          <input v-model.trim="spec.date_parameters.day" class="form-control mb-1" placeholder="day parameter (optional)">
        </template>
        <template v-else-if="!spec.use_page_title">
          <label class="form-label">Parameter <small class="text-muted">(and aliases; unnamed ones are 1, 2, …)</small></label>
          <div v-for="(p, i) in spec.parameters" class="input-group mb-1">
            <input v-model.trim="spec.parameters[i]" class="form-control" :placeholder="i ? 'alias' : 'parameter'">
            <button v-if="spec.parameters.length > 1" type="button" class="btn btn-outline-secondary" @click="spec.parameters.splice(i, 1)" title="remove">×</button>
          </div>
          <button type="button" class="btn btn-sm btn-link px-0" @click="spec.parameters.push('')">+ add alias</button>
          <input v-model="spec.value_pattern" class="form-control form-control-sm font-monospace mt-1" placeholder="or combine parameters: {1}-{2}">
          <div v-if="datatype === 'globe-coordinate'" class="form-text"><span v-pre>A nested {{coord|…}} is read. An unnamed parameter takes the following unnamed ones along, for harvesting {{coord}} itself.</span></div>
        </template>
        <div class="form-check mt-1">
          <input type="checkbox" class="form-check-input" id="pagetitle" v-model="spec.use_page_title">
          <label class="form-check-label" for="pagetitle">use the page title instead</label>
        </div>
        <div class="form-check">
          <input type="checkbox" class="form-check-input" id="unwrap" v-model="spec.unwrap_templates">
          <label class="form-check-label" for="unwrap">use the content of nested templates: <code v-pre>{{URL|x}}</code> → x</label>
        </div>

        <template v-if="datatype === 'wikibase-item'">
          <div class="form-check">
            <input type="checkbox" class="form-check-input" id="plain" v-model="spec.plain_links">
            <label class="form-check-label" for="plain">match pages even without [[link]] syntax</label>
          </div>
          <label class="form-label mt-2">With several links, use the</label>
          <select v-model="spec.link_choice" class="form-select"><option value="first">first</option><option value="last">last</option></select>
        </template>
        <template v-if="datatype === 'time'">
          <label class="form-label mt-2">Calendar</label>
          <select v-model="spec.calendar" class="form-select"><option value="gregorian">Gregorian</option><option value="julian">Julian</option></select>
          <div class="form-check mt-2">
            <input type="checkbox" class="form-check-input" id="limit" :checked="!!spec.date_limit" @change="setLimit($event.target.checked)">
            <label class="form-check-label" for="limit">only dates</label>
          </div>
          <div v-if="spec.date_limit" class="input-group input-group-sm">
            <select v-model="spec.date_limit.relation" class="form-select ht-short"><option value="at_least">from</option><option value="before">before</option></select>
            <input v-model="limitText" class="form-control" placeholder="1582-10-15">
          </div>
        </template>
        <template v-if="datatype === 'quantity'">
          <label class="form-label mt-2">Unit</label>
          <select v-if="property?.units" v-model="spec.unit" class="form-select">
            <option v-for="u in property.units" :value="u.id">{{ u.label }}</option>
          </select>
          <input v-else class="form-control" :value="spec.unit ?? ''" @input="spec.unit = $event.target.value.trim() || null" placeholder="Q-id, empty for no unit">
          <label class="form-label mt-2">Decimal mark</label>
          <select v-model="spec.decimal_mark" class="form-select"><option value=".">. (1,234.5)</option><option value=",">, (1.234,5)</option></select>
        </template>
        <template v-if="datatype === 'url'">
          <label class="form-label mt-2">Links to archived copies (web.archive.org, …)</label>
          <select v-model="spec.archive_urls" class="form-select">
            <option value="original">use the original URL</option>
            <option value="skip">skip them</option>
            <option value="keep">keep them</option>
          </select>
        </template>
        <template v-if="datatype === 'monolingualtext'">
          <label class="form-label mt-2">Language code</label>
          <input v-model.trim="spec.language" class="form-control" placeholder="en">
        </template>

        <h2 class="h5 mt-4">Qualifiers</h2>
        <qualifier-editor v-model="spec.qualifiers" />

        <h2 class="h5 mt-4">Modify values</h2>
        <div class="row g-1">
          <div class="col-6"><input v-model="spec.transform.add_prefix" class="form-control form-control-sm" placeholder="add prefix"></div>
          <div class="col-6"><input v-model="spec.transform.add_suffix" class="form-control form-control-sm" placeholder="add suffix"></div>
          <div class="col-6"><input v-model="spec.transform.remove_prefix" class="form-control form-control-sm" placeholder="remove prefix"></div>
          <div class="col-6"><input v-model="spec.transform.remove_suffix" class="form-control form-control-sm" placeholder="remove suffix"></div>
          <div class="col-6"><input v-model="spec.transform.search" class="form-control form-control-sm font-monospace" placeholder="regex search"></div>
          <div class="col-6"><input v-model="spec.transform.replace" class="form-control form-control-sm font-monospace" placeholder="replace ($1…)"></div>
          <div class="col-12">
            <select v-model="spec.transform.case" class="form-select form-select-sm" aria-label="case">
              <option value="unchanged">keep the case</option>
              <option value="lower">lower case</option>
              <option value="upper">upper case</option>
            </select>
          </div>
        </div>
      </div></div>
    </section>

    <section class="col-xl-3 col-md-6">
      <div class="card h-100"><div class="card-body">
        <h2 class="h5">Filter</h2>
        <label class="form-label">Category</label>
        <input v-model.trim="spec.category" class="form-control" placeholder="optional">
        <label class="form-label mt-2">Category depth</label>
        <input v-model.number="spec.depth" type="number" min="0" max="30" class="form-control ht-short">
        <label class="form-label mt-2">Only these pages or items</label>
        <textarea v-model.lazy="manualListText" class="form-control" rows="4" placeholder="one title or Q-id per line"></textarea>
        <label class="form-label mt-2">Only pages in PetScan query</label>
        <input :value="spec.petscan ?? ''" @change="spec.petscan = Number($event.target.value) || null" type="number" min="1" class="form-control" placeholder="PSID, optional">
        <label class="form-label mt-2">Only items selected by SPARQL</label>
        <textarea v-model.lazy="spec.sparql" class="form-control font-monospace small" rows="2" placeholder="SELECT ?item { ?item wdt:P31 wd:Q5 }"></textarea>
        <label class="form-label mt-2">Only items that are instances of</label>
        <input v-model.lazy="instanceOfText" class="form-control" placeholder="Q571, … (subclasses included)">
        <div class="form-check mt-2">
          <input type="checkbox" class="form-check-input" id="lead" v-model="spec.lead_only">
          <label class="form-check-label" for="lead">only templates before the first heading</label>
        </div>
        <label class="form-label mt-3">Skip items that already have</label>
        <select v-model="spec.skip_if" class="form-select">
          <option value="property">any value for the property</option>
          <option value="value">this exact value</option>
        </select>
        <div class="form-check mt-2">
          <input type="checkbox" class="form-check-input" id="removed" v-model="spec.skip_removed">
          <label class="form-check-label" for="removed">skip values someone removed from the item before</label>
        </div>
      </div></div>
    </section>

    <section class="col-xl-3 col-md-6">
      <div class="card h-100"><div class="card-body">
        <h2 class="h5">Check constraints</h2>
        <p v-if="!property" class="text-muted small">Choose a property to see its constraints.</p>
        <p v-else-if="!property.constraints.length" class="text-muted small">The property has no constraints.</p>
        <div v-for="c in property?.constraints ?? []" class="form-check">
          <input type="checkbox" class="form-check-input" :id="'c-' + c.id" :checked="constraintChecked(c)"
                 :disabled="c.status !== 'normal' || !c.supported" @change="toggleConstraint(c.id)">
          <label class="form-check-label" :for="'c-' + c.id">
            <a :href="entityUrl(c.id)" target="_blank" rel="noopener" class="text-reset">{{ c.label }}</a>
            <span v-if="c.status === 'mandatory'" class="badge text-bg-warning ms-1">mandatory</span>
            <span v-else-if="c.status === 'suggestion'" class="badge text-bg-light ms-1">suggestion</span>
            <span v-else-if="!c.supported" class="badge text-bg-light ms-1">not checked</span>
          </label>
        </div>
      </div></div>
    </section>
  </div>

  <div class="ht-footer d-flex flex-wrap align-items-center gap-3 mt-3">
    <button type="submit" class="btn btn-primary btn-lg" :disabled="busy">{{ busy ? 'Loading pages…' : 'Load pages' }}</button>
    <span v-if="!user" class="text-muted">You will be asked to log in.</span>
    <a :href="permalink" class="ms-auto">Permalink</a>
    <div v-if="error" class="alert alert-danger py-2 mb-0 w-100">{{ error }}</div>
  </div>
</form>`,
};
