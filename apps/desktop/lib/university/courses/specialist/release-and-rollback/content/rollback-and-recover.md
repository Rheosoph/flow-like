Simulate a rejected release by deciding that the candidate's message is wrong for its caller. Keep both snapshots unchanged.

1. Edit the original practice Event and select the previously tested version that prints `release A`.
2. Save, invoke and inspect the executed version and output. Expect `release A` again.
3. Leave the draft available for repair. Create a new candidate when the fix is ready; do not edit a numbered snapshot in place.
4. Disable the temporary test Event when finished.

Rollback by repointing changes which flow future invocations execute. It does not undo a message already sent, a database migration or a write produced by the rejected version. Plan separate data recovery or compensating work for those effects.

Record the old target, candidate, reason for rollback, restored target and verification. For an external dependency or schema change, first check that the previous flow can still run against the current environment.
