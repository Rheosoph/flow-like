# Explore browser fixture

Run `bunx vite --config tests/explore-browser/vite.config.ts` from the repository root, then open `http://127.0.0.1:4326/store/explore/apps`.

With the fixture server running, run the browser checks from the repository root:

```sh
node tests/explore-browser/verify.mjs
node tests/explore-browser/verify-layout.mjs
node tests/explore-browser/verify-desktop.mjs
```

Set `CHROME_EXECUTABLE_PATH` to use a specific Chrome executable. On macOS, the checks default to Google Chrome in `/Applications`.

The layout checks verify that the header scrolls with the page, Apps and Packages align across tabs, and controls keep their positions while data loads or filters change.

The desktop checks mount the actual desktop Apps and Packages route components. They verify one Explore heading, matching header and content widths, Explore/Installed tab navigation, visible installed package titles, and no horizontal overflow at 320, 390, and 1440 pixels. Screenshots are saved to `/private/tmp/explore-desktop-packages-<width>.png` and `/private/tmp/explore-desktop-installed-<width>.png`.

The fixture renders the shared `ExploreAppsPage` with React Query and a local mock backend. It contains 64 apps across eight categories and two suites. Icons are inline SVG images. Search, category filters, sort order, and offset pagination use the actual backend method arguments. The initial 50 results leave 14 apps for the next page.

Open `http://127.0.0.1:4326/store/packages?developer` to verify the shared Packages page and its `PackageListContent`. Its mock registry contains 23 packages, with 12 on the first page and 11 on the second. Search, category, verification, sorting, and pagination parameters are applied to the fixture data. Registry requests appear in the same request log as app requests.

Use `http://127.0.0.1:4326/store/packages?desktop&developer` for the actual desktop route, including its Explore and Installed views. This mode mocks desktop API requests, native registry commands, compile status, and authentication. It returns three installed remote packages and compiles the desktop route’s Tailwind classes. The production route and layout components render unchanged.

Use these query flags individually or together when opening the fixture:

- `?empty`: return no apps, suites, or packages.
- `?error`: fail catalog requests.
- `?loading`: hold catalog requests until **Restore service** is clicked.
- `?developer`: enable the Packages navigation tab.
- `?desktop`: render the actual desktop Apps and Packages route components. Pair with `&developer` to show both catalog tabs.
- `?runnable`: give owned apps an active event with a default page for direct launch checks.
- `?slow-suites`: delay suite results by two seconds while app results load normally.
- `?slow-apps`: delay app results by two seconds while suite results load normally.
- `?slow-packages`: delay registry results by two seconds.
- `?deferred-profile`: hold the Packages profile query until **Restore service** is clicked, to inspect loading before registry search begins.
- `?light`: render the light theme.

Normal filter URLs also work, for example `?category=Productivity&q=notes&sort=rated`. Updating filters preserves fixture flags in the URL.

Open **Fixture requests** to inspect method arguments and response status. **Restore service** clears an outage or releases held requests; use the page’s retry action to refetch a failed request. Browser checks can inspect `window.exploreQa`, call `window.exploreQa.restore()`, or change its `error`, `loading`, and `empty` properties before the next request.

The navigation shim updates browser history and preserves back/forward behavior. App, suite, and package detail destinations show a local navigation confirmation with a return action.
