Translate the test operator's job into permissions. The operator must run the existing Flow, trigger its Event and read logs, while leaving definitions unchanged.

| Group | Level for this job |
| --- | --- |
| Workflows | Run: ReadBoards and ExecuteBoards |
| Events | Trigger: ListEvents, ReadEvents and ExecuteEvents |
| Observability | Logs: ReadLogs |

Each ladder level includes lower levels within that group. Workflows Run does not itself grant Event execution. Use Full observability only when the job also needs analytics.

Owner and Admin are elevation flags that satisfy ordinary permission checks. A role name such as Operator does not establish its actual permissions: inspect the effective flags.

For a table-only reader, the raw `ReadDatabase` flag is narrower than `ReadFiles`, which also implies database read. If the standard Files & Data ladder cannot express the narrower job, use a custom role and document that choice. The test matrix checks file access as well as table access.

Completion: write the intended flags for both roles before creating them. Leave Owner/Admin absent from the test accounts. See [sharing and roles](https://docs.flow-like.com/apps/share/) for the UI reference.
