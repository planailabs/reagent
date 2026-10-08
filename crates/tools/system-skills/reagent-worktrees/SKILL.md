---
name: reagent-worktrees
description: Working on a branch of one's own with git worktrees in reagent - start, diff, merge (approval, strategies, conflicts), drop.
---

# Worktrees

Bigger or risky changes in a git project go into a worktree: a branch and a
checkout of the task's own, so the project folder (and other tasks) aren't
disturbed until the work is merged.

- `git.worktree_start(branch?, base?)`: a branch `reagent/<title>-<id>` from
  the project's current branch (or `base`); the task's working directory
  moves there. Worktrees live in reagent's data folder, or in `.worktrees/`
  in the project (a project setting).
- Commit there as you go (`git add`, `git commit` run without asking by
  the starter rules).
- `git.worktree_status()`, `git.worktree_diff(stat?)`: what's changed.
- `git.worktree_merge(strategy?, message?)`: `merge`, `squash` or `rebase`
  into the base branch. The worktree must be clean. With the project's
  `merge = approve` (the default) the person sees the diff and merges or
  sends it back with a message (you get it as the call's result). Conflicts
  are aborted and named: resolve them in your worktree (merge the base in),
  commit, and merge again.
- `git.worktree_drop(force?)`: throw it away (refused while it has unmerged
  commits, unless forced).

A merge waiting for the person survives a restart of reagent.
