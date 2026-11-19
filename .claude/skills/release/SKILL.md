---
name: release
description: Release a new IsctCompanion version by passing Release checks on release/X.Y.Z and merging it to main for automatic tagging. Use when the user asks to release, cut, tag or mark a version ("发布 1.1.8", "标记为 1.1.8", "release a patch").
---

# Release a desktop version

Merge a verified `release/X.Y.Z` to `main` to publish a new IsctCompanion
version. GitHub tags the squash commit and starts the desktop and mobile
release workflows after it verifies the candidate.

## 1. Start from an up-to-date main

```bash
git fetch origin
git status --short
```

Work in the main checkout on `main`, level with `origin/main` (neither ahead
nor behind), then `git switch -c release/<version>`. Files the user has changed
but not committed are theirs: leave them alone and stage only by path in step 5.

## 2. Bump

The user names the version; ask if they did not. A fix release is a patch
(`1.1.7` → `1.1.8`).

```bash
yarn release:bump <version>
```

`scripts/release/bump.mjs` rewrites every place the release validates (both
`package.json`s, `tauri.conf.json` version and macOS `bundleVersion`,
`Cargo.toml`, `Cargo.lock`), reads them back, and refuses a version that is not
newer than the files and the latest `v*` tag, or files that already disagree.
It prints the commits since the last tag. Never edit those six places by hand.
`yarn release:bump --check` lists them.

## 3. Write the release notes

Create `docs/releases/<version>.md`; the release workflow requires it. Read the
two previous files first and match them:

- one `- ` bullet per change a user would notice, in English;
- the most serious fix first;
- say what changed for the user, not how: no commit hashes, file names or
  internal type names;
- skip changes nobody can see (tests, refactors, tooling).

Build them from the commits the bump printed, reading a commit's body when its
subject is not enough. Show the user the notes before committing.

## 4. Validate the version and notes locally

```bash
GITHUB_REF=refs/tags/v<version> node scripts/desktop/desktop-release.mjs validate
```

If `docs/verification/manual-checks.yaml` changed since the last tag, run
`node scripts/checks/manual-acceptance.mjs render` and include the regenerated
`docs/verification/manual-acceptance.md`. The full tests run in Release checks.

## 5. Commit

```bash
git add Cargo.lock package.json apps/desktop/package.json \
  apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/tauri.conf.json \
  docs/releases/<version>.md   # plus any fix the gate needed
git commit -m "Release IsctCompanion <version>"
```

The commit carries the user's name only: no `Co-Authored-By` or other AI
attribution line.

## 6. Verify the release branch and merge

```bash
git push -u origin release/<version>
gh workflow run release-checks.yml --ref release/<version>
```

Wait for **Release checks** on the release branch commit. After it passes,
open the release PR to `main` with the user's approval.

```bash
gh pr create --base main --head release/<version> \
  --title "Release IsctCompanion <version>" --body-file docs/releases/<version>.md
```

If the PR already exists, rerun the failed Route job after Release checks
passes. If the branch content changes, run Release checks again on the updated
commit. Squash merge only after the required PR checks pass.

## 7. Watch automatic tagging and publishing

Check **Tag merged release** first, then the three platform release workflows
it starts on `v<version>`. Report the run links and any failed check. Retry a
failed build or upload job on the same version. If source needs to change
after the tag is created, ship the change in the next patch version.
