// Root component: header, login state and hash routes.
//   #/          the harvest form (permalink parameters in the query string)
//   #/run/ID    one run
//   #/runs      my runs
//   #/shares    shared queries; #/shares/TAG only those with that tag
import { createApp, ref, onMounted } from 'vue';
import HarvestForm from './js/harvest-form.js';
import RunView from './js/run-view.js';
import RunsList from './js/runs-list.js';
import SharesList from './js/shares-list.js';
import { user, refreshUser, login, logout } from './js/session.js';

const DOCS = 'https://github.com/magnusmanske/harvesttemplates/blob/main/docs';

function parseHash() {
  const [, view = '', param = ''] = location.hash.split('/');
  return { view, param: decodeURIComponent(param) };
}

const App = {
  components: { HarvestForm, RunView, RunsList, SharesList },
  setup() {
    const route = ref(parseHash());
    const ready = ref(false);
    const menuOpen = ref(false);
    window.addEventListener('hashchange', () => {
      route.value = parseHash();
      menuOpen.value = false;
    });
    onMounted(async () => {
      await refreshUser();
      ready.value = true;
    });
    return { route, ready, menuOpen, user, login, logout, DOCS };
  },
  template: `
<header class="ht-header">
  <div class="container-fluid d-flex flex-wrap align-items-center column-gap-3">
    <a href="/" class="ht-brand">Harvest Templates</a>
    <span class="ht-tagline d-none d-lg-inline">from Wikimedia templates to Wikidata</span>
    <button type="button" class="ht-burger d-md-none ms-auto" @click="menuOpen = !menuOpen"
            :aria-expanded="menuOpen" aria-controls="ht-menu" aria-label="menu">☰</button>
    <nav id="ht-menu" class="ht-menu ms-md-auto" :class="{ open: menuOpen }">
      <a href="#/runs">My runs</a>
      <a href="#/shares">Shared queries</a>
      <a :href="DOCS + '/EXAMPLES.md'" target="_blank" rel="noopener">Examples</a>
      <a :href="DOCS + '/HELP.md'" target="_blank" rel="noopener">Help</a>
      <span v-if="user">{{ user }} · <a href="" @click.prevent="logout">log out</a></span>
      <a v-else href="" @click.prevent="login">Log in</a>
    </nav>
  </div>
</header>
<main class="container-fluid py-3">
  <template v-if="ready">
    <run-view v-if="route.view === 'run'" :id="Number(route.param)" />
    <runs-list v-else-if="route.view === 'runs'" />
    <shares-list v-else-if="route.view === 'shares'" :tag="route.param" />
    <harvest-form v-else />
  </template>
</main>`,
};

createApp(App).mount('#app');
