Local loading completes the core course. Publish only when another user needs a registry version and you have the required package access.

1. From a clean project state, run tests and build. Confirm the manifest version and artifact path.
2. Open the package workspace under **Packages → Mine**. Review its node definitions, capability list and build evidence before using the publish/release action.
3. Publish a new version and keep distribution private for the initial integration test. Test the consuming app's linked package version as well as the installed device copy.
4. Open a board saved with the previous release, if one exists, and inspect both execution and stored wiring.
5. Use the registry's publication/review process when requesting public availability.

Do not overwrite an already published package ID/version with a different binary. Create a new version. A permission increase and a renamed pin both deserve explicit review, even when a build is green.

Package publication, device installation and an app's dependency selection are different steps. Updating one does not prove every consumer uses the intended version.
