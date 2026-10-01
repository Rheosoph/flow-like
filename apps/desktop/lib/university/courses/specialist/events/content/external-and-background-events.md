Use the log-only Practice action flow from the first lesson. It is safe to invoke repeatedly.

1. Create a **Cron** Event on its Simple Event node.
2. Choose a one-time future date in the Event editor. Select a time a few minutes ahead and read the next-run preview.
3. Select an execution location your deployment supports. For a local test, keep Desktop open.
4. Activate the Event, wait for the occurrence and find its run. Confirm the log message and runtime location.
5. Disable this test Event when finished.

For a recurring schedule, use an IANA timezone such as `Europe/Berlin`. `0 9 * * 1` means Monday at 09:00 in that timezone. A local scheduler cannot run while Desktop is stopped; do not assume it replays missed occurrences. Hybrid can register local and remote schedules, so duplicate delivery must be considered.

Before scheduling work with side effects, decide what happens if runs overlap or a request is delivered twice. The Automation Recipes and API partial-failure labs practice these cases. A Daemon is appropriate for a persistent local process, such as a connection loop; a finite periodic job is easier to inspect as a scheduled run.
