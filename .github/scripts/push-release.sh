#!/usr/bin/env bash
# Pushes a release commit and its tag to main, then merges the commit back into
# develop so the two branches never disagree on a version.
#
#   push-release.sh <tag>
#
# Expects HEAD to be the release commit, already tagged <tag>, on top of main.
# Run only after the registry accepted the release: the push is what makes the
# version real in git, and a registry upload cannot be taken back.
set -euo pipefail

tag="${1:?usage: push-release.sh <tag>}"

# main may have moved while the workflow ran; replay the release commit on top
# and move the tag with it.
git pull --rebase origin main
git tag -f -a "${tag}" -m "${tag}"
git push origin HEAD:main "refs/tags/${tag}"

git fetch origin develop
git checkout -B develop origin/develop
git merge --no-edit "${tag}"
git push origin develop
