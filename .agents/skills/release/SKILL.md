---
name: release
description:
  "Cut a kladde release: bump versions in lockstep, tag, and watch the pipeline.
  Use only when the maintainer explicitly asks for a release; never on your own
  initiative."
metadata:
  internal: true
---

# Releasing kladde

Only run this when the maintainer explicitly asks for a release, never as an
automatic follow-on to a merge, and never by running a publish command locally:
every registry publish goes through the tag-triggered `release.yml` workflow
with OIDC.

## Steps

1. Bump the version in `Cargo.toml` and `pyproject.toml`. The two must match;
   the publish jobs hard-fail on a mismatch. Run `cargo check` once so
   `Cargo.lock` picks up the new version.
2. Run the full gate suite from AGENTS.md. All seven must pass.
3. Commit as `chore: Bump version to X.Y.Z` and land it on `main` through the
   normal PR flow.
4. Draft release notes from `git log <previous-tag>..HEAD`: hand-written,
   grouped under `### Features` and `### Bug Fixes`, user-facing impact only,
   omitting chore, style, test, and CI commits. Get maintainer sign-off on the
   text before creating anything.
5. Verify the pre-tag state: `git fetch origin` first, then working tree clean,
   on `main` with `HEAD` equal to `origin/main`, the new version in
   `Cargo.toml`, `pyproject.toml`, and `Cargo.lock`, and the tag absent both
   locally (`git tag -l vX.Y.Z`) and remotely
   (`git ls-remote --tags origin vX.Y.Z`).
6. Create the draft release targeting the exact commit, then push the tag:

   ```sh
   gh release create vX.Y.Z --draft --target "$(git rev-parse HEAD)" --title "kladde vX.Y.Z" --notes-file <file>
   git tag vX.Y.Z && git push origin vX.Y.Z
   ```

   The tag triggers `release.yml`: dist builds every target, uploads the
   installers into the draft (`create-release = false` in dist-workspace.toml),
   undrafts it once everything is attached, then publishes to crates.io and
   PyPI. Nothing is public until the artifacts are on the release.

7. Watch the run for the tag commit, not the latest run (`release.yml` also runs
   on pull requests):

   ```sh
   gh run list --workflow=release.yml --event push --commit "$(git rev-parse vX.Y.Z^{commit})"
   gh run watch <run-id>
   ```

## Failure modes

- Version mismatch: the publish jobs compare the planned version against
  `Cargo.toml` and `pyproject.toml` and fail before uploading anything.
- Tag pushed with no draft waiting: the host job fails. Create the draft, then
  re-run the failed job from the run page.
- Host job failed after uploading some assets: a plain re-run hits "asset
  already exists" errors, because its upload does not overwrite. Delete the
  uploaded assets first (`gh release delete-asset vX.Y.Z <name>` per file, or
  delete and recreate the draft), then re-run the failed job.
- Registry auth failures also happen before any upload. Fix the cause and re-run
  the failed job from the run page rather than re-tagging.
- Partial publication: the crates.io and PyPI jobs are independent, so one
  registry can succeed while the other fails. Never reuse the version or rewrite
  the tag; re-run the failed job, and if the artifacts themselves must change,
  cut the next patch version.
