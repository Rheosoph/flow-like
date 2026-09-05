# Deploying editable home layouts

Apply the home layout schema before starting an API build that reads the new profile fields. The migration adds nullable `homeLayout` and `homeDefaultId` columns to `Profile`, plus a `HomeDefault` table for published layouts. Existing profile layouts remain unset and follow the published default.

The Aurora DSQL deployment migration job applies `migrations-dsql/20260905103000_profile_home_layouts/migration.sql` through its migration ledger. The file passes `dsql-lint`.

For an existing PostgreSQL installation that has not applied the schema change, run this command from `packages/api` with `DATABASE_URL` pointing to that installation:

```sh
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f prisma/migrations/20260905103000_profile_home_layouts/migration.sql
```

Apply the migration once. Fresh PostgreSQL installations can use the existing `db:push` workflow, which reads the updated Prisma schema. If an existing installation uses `db:push` instead of the SQL migration, run the migration's final `UPDATE` separately to associate existing profiles with matching template IDs.

The backfill associates a profile with a template only when their IDs match. Profiles created by copying another profile may have a different ID and no recoverable template reference. New template installations carry an explicit reference, and new copies retain it.

After deployment, an administrator with `WriteLandingPage` or `Admin` permission can publish the main default and optional template defaults. Publishing checks the revision loaded when editing began. A conflicting publication returns HTTP 409 so the editor can retain the draft and ask the administrator to reload.

Resetting a user's home clears only `homeLayout`; `homeDefaultId` remains intact. The profile then follows its latest template default, falls back to the main default, and finally uses the bundled layout if no published default is available. Removing a template default restores the main default. Removing the main default restores the bundled layout.

Desktop custom layouts are saved locally and synchronized with the profile. Older clients that omit the home fields in partial API writes leave them unchanged. Default configuration contains widget settings, while each viewer loads app data with their own access permissions.
