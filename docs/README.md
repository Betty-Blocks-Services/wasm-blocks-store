# docs/

Shared, living knowledge about WASM Actions on Betty Blocks — not just the steps themselves
(see `../functions/`). If you open Claude Code inside a clone of this repo, `.claude/skills/
wasm-actions/SKILL.md` reads this automatically and tells Claude how to keep it up to date;
read that file if you want to know the mechanics, this one is just the index.

```
crash-course/
  wasm-actions-crash-course.md   Architecture & platform guide: what a WASM Action is, how to
                                   build one from scratch with the bb CLI, function.json
                                   reference, packaging/publishing, known issues.
  rust-crash-course.md            A Rust primer scoped to exactly what these components use —
                                   not general Rust. Written for zero prior Rust experience.
testing-a-step-in-a-real-app.md  How to actually publish and test a step from this repo on a
                                   real Betty Blocks app — the "actually published and tested
                                   live" checklist item in CONTRIBUTING.md.
product-feedback-log.md          Confirmed findings worth reporting to Betty Blocks' own
                                   product developers (bugs/gaps in the WASM platform itself).
developer-learnings-log.md       Confirmed findings worth other BB developers knowing (gotchas,
                                   tooling quirks, corrected assumptions) — feeds back into the
                                   crash-course docs above over time.
wasco-dev.md                      A separate, public, generic WASM component registry (not
                                   Betty Blocks tooling) — what it is, how to reuse an existing
                                   component inside a BB app, and when a new step should land
                                   there instead of here.
```

Both logs are living documents — newest entries at the top, each one dated and either
confirmed, open, or (a few times already) explicitly retracted once a claim didn't hold up.
That history is kept, not deleted, on purpose: it's the record of what's actually been checked
versus what only sounded right at the time.
