// Links to wikis and Wikidata, and small formatting helpers.
export const pageUrl = (host, title) => `https://${host}/wiki/${encodeURIComponent(title.replaceAll(' ', '_'))}`;
export const entityUrl = (id) => `https://www.wikidata.org/wiki/${id.startsWith('P') ? 'Property:' : ''}${id}`;
export const editGroupUrl = (editgroup) => `https://editgroups.toolforge.org/b/harvesttemplates/${editgroup}/`;


export function formatTime(unix) {
  return unix ? new Date(unix * 1000).toLocaleString() : '';
}

/** A link for a harvested value, by datatype; `null` if there is none. */
export function valueUrl(property, value) {
  if (!property || !value) return null;
  const main = value.split(/; P\d+: /)[0];
  const encoded = main.split('/').map(encodeURIComponent).join('/');
  switch (property.datatype) {
    case 'wikibase-item': return entityUrl(main);
    case 'commonsMedia': return `https://commons.wikimedia.org/wiki/File:${encodeURIComponent(main.replaceAll(' ', '_'))}`;
    case 'url': return /^https?:\/\//.test(main) ? main : null;
    case 'external-id': return property.formatter_url?.replaceAll('$1', encoded) ?? null;
    default: return null;
  }
}

/** How a row status reads to people. */
export const rowStatusLabel = (status) => ({ ready: 'would add', done: 'added' })[status] ?? status;

/** A Bootstrap colour for a run or row status. */
export function statusColor(status) {
  return {
    done: 'success', ready: 'info', skipped: 'secondary', error: 'danger', pending: 'light',
    loading: 'warning', previewing: 'warning', editing: 'primary', paused: 'secondary', failed: 'danger',
  }[status] ?? 'light';
}
