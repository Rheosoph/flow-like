# Starter profile management

Administrators can curate the profiles offered to new users at `/admin/profiles` in the web and desktop apps. The old `/admin/user` and `/admin/user/edit` routes open the same manager.

The manager supports searching, sorting, creating, editing, duplicating, and deleting templates. The editor has a live profile preview and sections for the name, description, icon, cover, tags, interests, bits, apps, hubs, and flow connection style. Included apps retain their favorite and pin settings. Editing a template does not rewrite existing user profiles.

Each saved template links to `/admin/home?default=<template-id>`. Its home follows the main default until an administrator publishes a template default. Duplicating a template copies its presentation and starting configuration, while its new home initially follows the main default. Personal layouts, shortcuts, and private bits are not copied. A published template home must be reset to follow the main default before deleting its template.

Both apps use the shared editor. Desktop supplies a native HTTP upload function; the browser uses Fetch. Image uploads accept PNG, JPEG, and WebP up to 10 MiB and 4096 pixels per edge. Images are converted to WebP when the browser can encode it, with a PNG fallback for other engines, keeping their proportions, with a longest edge of 512 pixels for icons or 1600 pixels for covers. Administrators can also supply an HTTP(S) image URL. Failed uploads keep the previous image.

`WriteProfile` controls template edits and media uploads. `ReadProfile` allows browsing, and `WriteLandingPage` separately controls default homes. These administration operations require a reachable backend and its permissions, including in the desktop app. Desktop users can still use their local profiles without signing in.

Image upload URLs accept `?format=webp`, `png`, or `jpeg` and use the corresponding extension; requests without a format keep the existing WebP behavior. Template writes use the requested ID and return the saved profile. Validation bounds names to 120 characters, descriptions to 10,000 characters, bits and apps to 500 entries each, and tags, interests, and additional hubs to 50 entries each. Stored apps, settings, and theme values survive template round trips. No new database migration is required for this manager; the default home feature has its own migration.

Run the focused frontend checks with `bun test packages/ui/components/profile-templates`. The responsive browser fixture and verification script live in `tests/home-browser`.
