#!/usr/bin/env bash
set -euo pipefail

fail() { echo "$*" >&2; exit 1; }

release_version() {
  [[ $1 =~ ^release/([0-9]+\.[0-9]+\.[0-9]+)$ ]] ||
    fail "Expected a release/X.Y.Z branch, got $1."
  printf '%s\n' "${BASH_REMATCH[1]}"
}

commit_tree() {
  [[ $1 =~ ^[0-9a-f]{40}$ ]] || fail "Invalid commit SHA: $1"
  git rev-parse --verify "$1^{tree}" 2>/dev/null ||
    gh api "repos/$GH_REPO/git/commits/$1" --jq '.tree.sha'
}

verify_candidate() {
  local branch=$1 candidate=$2 release_sha=$3 run candidate_tree release_tree
  release_version "$branch" >/dev/null
  candidate_tree=$(commit_tree "$candidate")
  release_tree=$(commit_tree "$release_sha")
  [[ -n "$candidate_tree" && "$candidate_tree" == "$release_tree" ]] ||
    fail "Merged source differs from the release candidate. Update the release branch from main and verify the resulting release content."
  run=$(gh api "repos/$GH_REPO/actions/workflows/release-checks.yml/runs" --method GET \
    -f branch="$branch" -f head_sha="$candidate" -f event=workflow_dispatch -f status=success \
    --jq '.workflow_runs[0].html_url // empty')
  [[ -n "$run" ]] || fail "Run Release checks on $branch at $candidate before publishing."
  echo "Release candidate verified: $run"
}

tag_commit() {
  local tag=$1 object
  [[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "Expected an existing vX.Y.Z release tag."
  object=$(gh api "repos/$GH_REPO/git/ref/tags/$tag" --jq '.object')
  while [[ $(jq -r '.type' <<< "$object") == tag ]]; do
    object=$(gh api "repos/$GH_REPO/git/tags/$(jq -er '.sha' <<< "$object")" --jq '.object')
  done
  jq -er 'select(.type == "commit") | .sha' <<< "$object"
}

case "$1" in
  version) release_version "$2" ;;
  candidate) verify_candidate "$2" "$3" "$4" ;;
  tag-commit) tag_commit "$2" ;;
  tag)
    tag=$2
    sha=$3
    [[ $(tag_commit "$tag") == "$sha" ]] ||
      fail "$tag does not point to the source commit $sha."
    branch="release/${tag#v}"
    candidate=$(gh api "repos/$GH_REPO/commits/$sha/pulls" --paginate | jq -sr \
      --arg branch "$branch" --arg repo "$GH_REPO" --arg sha "$sha" '
      [ .[][] | select(.merged_at != null and .merge_commit_sha == $sha and
        .base.ref == "main" and .head.repo.full_name == $repo and .head.ref == $branch) ][0].head.sha // empty')
    [[ -n "$candidate" ]] || fail "$tag must point to the merge of $branch into main."
    verify_candidate "$branch" "$candidate" "$sha"
    ;;
  *) fail "Usage: verify-candidate.sh version BRANCH | candidate BRANCH TESTED_SHA MERGED_SHA | tag-commit TAG | tag TAG SOURCE_SHA" ;;
esac
