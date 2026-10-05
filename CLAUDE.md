# HarvestTemplatesClaude project notes

## Hard rules

- **Do not add the Claude attribution to commit messages.** No `Co-Authored-By: Claude …` line.
- **`git commit` is pre-authorized; `git push` is not.** When work is complete and the diff is reviewable, you may run `git add <specific files>` + `git commit` without asking — this is durable authorization. **Never `git push`, `git push --force`, or otherwise publish commits to a remote without an explicit per-action request from the user.** Same for any other remote-publishing action (PR creation, branch deletion on origin, etc.). Use sensible commit groupings for larger changes.
- This is a web-facing product, so **keep security in mind**.
- Always keep **code readability** and **long-term maintenance** in mind.
- Adhere to **SOLID and DRY principles**.
- Use **best practices** and **language standards**.
- **Keep the code simple** and elegant.
- **Write tests** where it makes sense.
- **Aim to keep code small** where possible.
- **Warn me if you think what I ask of you is a bad idea.** Or just window dressing, with no improvement of functionality, UX, or code readability. Be honest.
- Fix clippy warnings, even pre-existing ones.
- Surgical Edits: "Prefer editing existing lines over rewriting entire blocks. Avoid adding middleware, frameworks, or abstract classes for simple feature requests".
- Complexity Ceiling: "Keep cyclomatic complexity low. If a function exceeds 10-15 lines, evaluate if it can be simplified before splitting it into smaller, potentially more fragmented functions".
- Discovery First: "Before creating a new utility or helper function, search the codebase to see if a canonical implementation already exists and reuse it".

## What this is

A rewrite of the "harvesttemplates" tool ([source](https://github.com/wmde/harvesttemplates), [live](https://pltools.toolforge.org/harvesttemplates/)) in Rust.
