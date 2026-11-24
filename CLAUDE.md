# Instructions for Claude Code in this repo

This repo is public and holds Betty Blocks WASM steps (`functions/`) and what we have learned
building them (`docs/`). Read [`README.md`](README.md) first.

## Reviewing a pull request

Any request to review, check, triage, re-review or merge a PR ("review PR 12", "check the open
PRs", "review the PRs at <url>", "/code-review <PR>") follows **one process: the
[`review-pr` skill](.claude/skills/review-pr/SKILL.md).** Invoke it with the Skill tool before
doing anything else, then follow it exactly, including its section 0 (what to read first and
which pass to run). Do not improvise a review, and do not use a generic review command on its
own: `code-review` is a step inside the skill, not a replacement for it.

- Review from a session whose working directory is a checkout of this repo: a local clone, or a
  cloud session on the repo. If it is not (a scratch folder, another project), stop and ask the
  user to open the session on this repo: the skill, `CONTRIBUTING.md` and the PR template are only
  available there.
- Read `CONTRIBUTING.md` and `.github/PULL_REQUEST_TEMPLATE.md` **in full, every run** (read the
  whole file, not the first screenful). The checklists in them are the source of truth; the
  summaries in the skill are not.
- Post findings only as a GitHub review in the skill's `agent-task-spec` format
  (`gh pr review --comment`), never as plain `gh pr comment`s, and only after the user says to
  post. Never push, merge or approve without being told to.

## Everything else

- Branches are `<type>/<short-kebab-description>` and commits are Conventional Commits; open a PR,
  never push to `main` (see `CONTRIBUTING.md`, "Git workflow").
- For any WASM step work, use the [`wasm-actions` skill](.claude/skills/wasm-actions/SKILL.md).
- This repo is public: never put customer names, app names, internal URLs or tokens in code, docs,
  PR text or comments.
