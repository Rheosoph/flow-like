# CI builds and caches

Pull requests to `dev` run the checks affected by their changed files. Rust tests,
Clippy, and feature checks also run on relevant pushes to `dev` to populate caches
that later PRs can restore. The push test job compiles test executables without
calling external test services. PRs execute the tests.

Rust cache families separate test, Clippy, lightweight feature, and catalog
feature builds. Only successful `dev` builds save those caches. PRs restore them
without saving copies scoped to individual PR merge refs. A new cache family is
cold until its first successful `dev` build. The dependency-only feature job keeps
a smaller download cache and runs before the two parallel compilation jobs.

The change classifier reads the complete Git diff, including deletions and both
sides of renames. Known frontend trees and prose skip Rust builds; unknown inputs
run them. If the comparison fails, every check is enabled. Existing required
check names remain available when work is skipped.

## Desktop releases

The frontend builds once and is shared across all five native platforms. Only
macOS ARM waits for the MLX helper. Native builds use `release-native.yml` for
signing, packaging, upload recovery, and compatibility checks. Updater metadata
is finalized after both native job groups succeed.

Frontend jobs cache Bun downloads and `.next/cache`. The unsigned MLX helper is
reused only when its source, resolved dependencies, build script, and Apple
toolchain match exactly. Native jobs install the small, separately locked
`release-tools` package into runner temporary storage. Keep its Tauri CLI version
aligned with the root `bun.lock` when upgrading the desktop CLI.

## Cache capacity

The September 3, 2026 release produced about 18.3 GiB of Rust caches. The following
PR produced another 8.3 GiB. A 50 GB repository cache limit allows the measured
working set and room for replacement archives and frontend caches.

The organization currently limits this repository to 10 GB. GitHub rejected the
50 GB update, and the available token lacks organization admin access. An
organization administrator must raise the allowed cache capacity before a
repository administrator can apply:

```sh
gh api --method PUT repos/Rheosoph/flow-like/actions/cache/storage-limit \
  -F max_cache_size_gb=50
gh api repos/Rheosoph/flow-like/actions/cache/storage-limit
```

The second command should return `max_cache_size_gb: 50`. Cache storage used above
GitHub's included allowance is billed. Until the ceiling is raised, capacity
eviction can still remove otherwise reusable caches.

## Validate workflow edits

From the repository root, with actionlint installed:

```sh
actionlint .github/workflows/*.yml
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s .github/scripts/tests -v
```

Both commands should exit successfully. Validate native signing and packaging on
the corresponding GitHub runners; local workflow linting does not execute them.
