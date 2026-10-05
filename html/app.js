// Root component: header, login state and hash routes.
//   #/          the harvest form (permalink parameters in the query string)
//   #/run/ID    one run
//   #/runs      my runs
//   #/shares    shared queries
import { createApp, ref, onMounted } from 'vue';
import HarvestForm from './js/harvest-form.js';
import RunView from './js/run-view.js';
import RunsList from './js/runs-list.js';
import SharesList from './js/shares-list.js';
import { user, refreshUser, login, logout } from './js/session.js';

function parseHash() {
  const [, view = '', id = ''] = location.hash.split('/');
  return { view, id: Number(id) };
}

const App = {
  components: { HarvestForm, RunView, RunsList, SharesList },
  setup() {
    const route = ref(parseHash());
    const ready = ref(false);
    window.addEventListener('hashchange', () => { route.value = parseHash(); });
    onMounted(async () => {
      await refreshUser();
      ready.value = true;
    });
    return { route, ready, user, login, logout };
  },
  template: `
<header class="ht-header">
  <div class="container-fluid d-flex flex-wrap align-items-center gap-3">
    <a href="/" class="ht-brand">Harvest Templates</a>
    <span class="ht-tagline d-none d-lg-inline">from Wikimedia templates to Wikidata</span>
    <nav class="d-flex gap-3 ms-auto">
      <a href="#/runs">My runs</a>
      <a href="#/shares">Shared queries</a>
      <a href="https://github.com/magnusmanske/harvesttemplates#readme" target="_blank" rel="noopener">Help</a>
      <span v-if="user">{{ user }} · <a href="" @click.prevent="logout">log out</a></span>
      <a v-else href="" @click.prevent="login">Log in</a>
    </nav>
  </div>
</header>
<main class="container-fluid py-3">
  <template v-if="ready">
    <run-view v-if="route.view === 'run'" :id="route.id" />
    <runs-list v-else-if="route.view === 'runs'" />
    <shares-list v-else-if="route.view === 'shares'" />
    <harvest-form v-else />
  </template>
</main>`,
};

createApp(App).mount('#app');
