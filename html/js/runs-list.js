// The logged-in user's runs.
import { ref, onMounted } from 'vue';
import { api } from './api.js';
import { user, login } from './session.js';
import { formatTime, statusColor } from './links.js';

export default {
  setup() {
    const runs = ref(null);
    const error = ref('');
    onMounted(async () => {
      if (!user.value) return;
      try {
        runs.value = await api('/runs');
      } catch (e) {
        error.value = e.message;
      }
    });
    return { runs, error, user, login, formatTime, statusColor };
  },
  template: `
<h1 class="h4">My runs</h1>
<p v-if="!user"><button class="btn btn-primary" @click="login">Log in</button> to see your runs.</p>
<div v-else-if="error" class="alert alert-danger">{{ error }}</div>
<p v-else-if="!runs" class="text-muted">Loading…</p>
<p v-else-if="!runs.length" class="text-muted">No runs yet. <a href="#/">Start one.</a></p>
<table v-else class="table table-sm table-hover">
  <thead><tr><th>Run</th><th>Created</th><th>Wiki</th><th>Property</th><th>Template</th><th>Status</th></tr></thead>
  <tbody>
    <tr v-for="r in runs" :key="r.id">
      <td><a :href="'#/run/' + r.id">{{ r.id }}</a></td>
      <td>{{ formatTime(r.created) }}</td>
      <td>{{ r.spec.siteid }}.{{ r.spec.project }}</td>
      <td>{{ r.spec.property }}</td>
      <td>{{ r.spec.template }}</td>
      <td><span class="badge" :class="'text-bg-' + statusColor(r.status)">{{ r.status }}</span></td>
    </tr>
  </tbody>
</table>`,
};
