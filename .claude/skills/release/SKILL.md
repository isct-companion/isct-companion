---
name: release
description: Release a new IsctCompanion desktop version - bump the version in every place, write the release notes, pass the release gate, commit, tag v<version> and push. Use when the user asks to release, cut, tag or mark a version ("发布 1.1.8", "标记为 1.1.8", "release a patch").
---

# Release a desktop version

A pushed `v<version>` tag starts the Desktop release workflow
(`.github/workflows/desktop-release.yml`). Its first job, "Verify release
source", fails the whole release on any red check, before a single build. So
everything that job runs is run here first.

The desktop version only. iOS and Android versions are not part of this.

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

## 4. Pass the release gate

The same commands as the "Verify release source" job, unfiltered. This is the
one place where the full suites are run, not targeted tests:

```bash
node scripts/desktop/desktop-release.mjs validate
yarn test
yarn test:types
cargo test -p app-core --locked
```

From a worktree, point `CARGO_TARGET_DIR` at the main checkout's `target/` so
dependencies are not rebuilt.

If `docs/verification/manual-checks.yaml` changed since the last tag, run
`node scripts/checks/manual-acceptance.mjs render` first and include the
regenerated `docs/verification/manual-acceptance.md`; `yarn test` fails when it
is stale (this is what broke the first push of 1.1.7).

Fix any failure before going on. Never tag a red gate.

## 5. Commit

```bash
git add Cargo.lock package.json apps/desktop/package.json \
  apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/tauri.conf.json \
  docs/releases/<version>.md   # plus any fix the gate needed
git commit -m "Release IsctCompanion <version>"
```

The commit carries the user's name only: no `Co-Authored-By` or other AI
attribution line.

## 6. Merge, tag and push

`main` takes a commit only from a squashed pull request whose `Platform checks`
passed on a branch level with `main`. Opening the pull request needs the user's
approval, each time.

```bash
git push -u origin release/<version>
gh pr create --base main --head release/<version> \
  --title "Release IsctCompanion <version>" --body-file docs/releases/<version>.md
```

Once the check has passed, squash the pull request. The squash creates the
commit to release, so tag `main` only after it. Before tagging, confirm the tip
of `origin/main` is the release commit.

```bash
gh pr merge release/<version> --squash
git switch main
git pull --ff-only origin main
git log -1 --format=%s
git tag -a v<version> -m "IsctCompanion <version>"
git push origin v<version>
git branch -D release/<version>
```

The merge and the tag push need no further approval. The branch needs `-D`:
after a squash its own commits are not in `main`.

## 7. Watch the verify job

```bash
gh run list --workflow "Desktop release" --branch v<version> --limit 1
gh run view <run-id> --json jobs --jq '.jobs[] | "\(.name): \(.status)/\(.conclusion)"'
```

Wait for "Verify release source" (a few minutes) and report it. The platform
builds take much longer; report that they are running and give the run link.

## If the release fails after the push

Read the failed step with `gh run view <run-id> --log-failed`, fix the cause,
and run the gate again. `main` refuses a force push, so the release commit cannot
be amended: land the fix through a pull request and ship it as the next patch
version.
