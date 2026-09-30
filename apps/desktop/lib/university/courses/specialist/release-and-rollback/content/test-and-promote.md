Use the changed draft from the previous lesson. A compatible correction usually warrants Patch; a compatible addition may warrant Minor; a change requiring caller migration may warrant Major. Choose based on the contract, not the size of the edit.

1. Run the draft and confirm `release B`.
2. Create a numbered candidate version and record it.
3. Create a temporary Quick Action targeting that candidate. Invoke through the action and inspect the executed version and output.
4. Check that the selected entry node still exists in the candidate. In a payload-bearing flow, test required fields, boundary values and an invalid request as well.
5. Repoint the original practice Event to the tested candidate and invoke again. Expect `release B`.

The candidate flow, Event configuration and dependencies form the tested path. A local Studio run does not establish that a remote Event has the required credentials, files or connections.

For a real rollout, decide what evidence to watch and what would trigger rollback before repointing callers. A snapshot freezes flow behavior, not an external API, database schema or provider credential.
