
- **Workflow actions are locked.** `.github/workflows/actions.lock` records the
  commit behind every action the workflows use, and the runner executes that
  commit. After adding a workflow or changing a `uses:` line, run `gh
  actions-lock` (the `github/gh-actions-lock` extension) and commit the lockfile
  with the workflow: a workflow whose entry is missing or stale does not start.
  Never edit the lockfile by hand.
