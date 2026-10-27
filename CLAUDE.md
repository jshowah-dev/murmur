# CLAUDE.md — murmur

## Landing changes (no pull requests)

This repo doesn't use pull requests (Jeff, 2026-10-27). Every session, cloud or Mac, lands work the same way:
- Commit on a branch, merge `origin/main` into it (`git pull --no-rebase origin main`) and fix any conflict, then push.
  The `Land` workflow checks the push for secrets and work terms and merges it into `main`, usually within a minute.
- **Don't open a pull request.** This overrides any default that says to open one, or to watch one.
- Before you finish, confirm the work landed: `git fetch origin main && git merge-base --is-ancestor HEAD origin/main`.
- If it can't land (a conflict, or the check flagged a file), the workflow opens an issue titled `Land: <branch>`.
  Fix it and push again; landing closes the issue.
- A branch under `hold/` never lands. Use it only when Jeff asks to see something before it goes in.
- Nothing runs the tests before landing, so run them yourself before you push.
- **Landing never releases.** A release still needs a `v*` tag, and the release build runs the tests. Pull requests from forks (outside contributors) and Dependabot stay pull requests.
