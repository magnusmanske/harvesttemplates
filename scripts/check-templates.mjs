// Compile every Vue template in html/ with Vue's own compiler, so a stray
// `{{…}}` or an unclosed tag fails CI instead of blanking the page.
// Usage: npm install --no-save @vue/compiler-dom && node scripts/check-templates.mjs
import { compile } from '@vue/compiler-dom';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const files = ['html/app.js', ...readdirSync('html/js').map((f) => join('html/js', f))];
let failures = 0;
for (const file of files) {
  const source = readFileSync(file, 'utf8');
  for (const [, template] of source.matchAll(/template:\s*`([\s\S]*?)`/g)) {
    compile(template, {
      prefixIdentifiers: true, // parse expressions too, like the runtime will
      onError: (e) => {
        failures += 1;
        console.error(`${file}: ${e.message}\n  near: ${template.slice(Math.max(0, e.loc?.start.offset - 40), e.loc?.start.offset + 40)}`);
      },
    });
  }
}
console.log(`${files.length} files checked, ${failures} template error(s)`);
process.exit(failures ? 1 : 0);
