#!/usr/bin/env bash
set -euo pipefail

# Reuse only a completed push of the exact source tree, with a plan recorded
# after its gate passed. The PR keeps its history-sensitive frontend checks.
[[ "$PR_HEAD_REPO" == "$GH_REPO" ]] || exit 1
[[ "$(git rev-parse "HEAD^{tree}")" == "$(git rev-parse "$PR_HEAD_SHA^{tree}")" ]] || exit 1

run=$(gh api "repos/$GH_REPO/actions/workflows/platform-checks.yml/runs" --method GET \
  -f branch="$PR_HEAD" -f head_sha="$PR_HEAD_SHA" -f event=push -f status=success -F per_page=1 \
  --jq '.workflow_runs[0] | select(. != null) | [.id, .run_attempt] | @tsv')
read -r id attempt <<< "$run"
[[ -n "$id" && -n "$attempt" ]] || exit 1

dir=$(mktemp -d)
trap 'rm -r "$dir"' EXIT
gh run download "$id" --name "platform-check-plan-$attempt" --dir "$dir"
cmp -s "$1" "$dir/platform-check-plan"
echo "Reusing source checks from successful push: $GITHUB_SERVER_URL/$GH_REPO/actions/runs/$id/attempts/$attempt"
