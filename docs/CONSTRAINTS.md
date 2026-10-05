# Constraint checks

Before each edit, the new statement is checked against the property's
constraints: all *mandatory* ones, plus the *normal* ones the user left
selected. *Suggestion* constraints are ignored. Exceptions (P2303) are honoured.

Local checks use the item fetched right before the edit; remote ones ask
another service. The registry is `constraints::TYPES`.

| Constraint | Item | Checked | How |
|---|---|---|---|
| allowed entity types | Q52004125 | local | must allow Wikibase items |
| allowed qualifiers | Q21510851 | local | |
| allowed units | Q21514353 | local | also validated when the run is created |
| citation needed | Q54554025 | local | always met: we add references |
| Commons link | Q21510852 | Commons API | page exists, in the right namespace |
| conflicts with | Q21502838 | local + WDQS | ours vs. the item, and other properties' constraints vs. ours |
| distinct values | Q21502410 | search | `haswbstatement`; WDQS for values with spaces |
| format | Q21502404 | local | `fancy-regex`, anchored, backtracking limit |
| integer | Q52848401 | local | |
| inverse | Q21510855 | WDQS | |
| item requires statement | Q21503247 | local | |
| mandatory qualifier | Q21510856 | local | |
| no bounds | Q51723761 | local | always met: we add no bounds |
| none of | Q52558054 | local | |
| one of | Q21510859 | local | |
| property scope | Q53869507 | local | must allow main values |
| range | Q21510860 | local | dates by every day they could mean; "unknown value" means now |
| single value | Q19474404 | local | |
| single best value | Q52060874 | local | |
| symmetric | Q21510862 | WDQS | |
| type | Q21503250 | local + WDQS | direct P31 first, subclasses via WDQS |
| value requires statement | Q21510864 | WDQS | |
| value type | Q21510865 | WDQS | |

## Known limitations

- WDQS lags behind Wikidata by seconds to minutes, and since the graph split its
  main endpoint lacks scholarly articles. Hence the local and search-based checks.
- A format pattern that `fancy-regex` cannot compile, or that exceeds the
  backtracking limit, does not block the edit (it is logged).
- "Separator" qualifiers (P4155) of single-value constraints are not considered.
