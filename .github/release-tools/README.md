# Desktop release tools

Native release jobs install these packages into the runner's temporary directory.
The frontend job installs the full workspace and shares its exported assets with
each native target.

Keep the Tauri CLI and dotenv CLI versions aligned with the versions resolved in
the root `bun.lock`. Update this directory's lockfile in a temporary directory
outside the repository so Bun does not discover the monorepo workspace. Commit
the resulting `package.json` and `bun.lock` together.

The native workflow explicitly runs `apps/desktop/scripts/sync-version.ts` because
it no longer installs the desktop workspace or runs its postinstall script.
