Use the fixed Trim String → To Lower Case flow. Run every case from `normalization-cases.json`:

| Input | Expected |
| --- | --- |
| `  Mixed CASE  ` | `mixed case` |
| `already clean` | `already clean` |
| spaces only | empty string |
| empty string | empty string |

Now create a Quick Action on the entry node and invoke through that action. Confirm the same result and inspect the Event's selected version. A Studio run alone does not test the Event configuration.

For a real repair, use the input that failed, an input that previously worked, a boundary case and a fresh input through the configured surface. Check the resulting file/record/message when an outcome exists beyond the run.

Before remote testing, inspect local-only dependencies, including those inside layers. A flow needing browser or desktop automation needs a compatible local execution environment. An Event runtime setting cannot make those dependencies appear on an unsupported worker.

Use Release and Rollback to promote a tested version and restore an earlier target when necessary. This lab has no production consumer to repoint.
