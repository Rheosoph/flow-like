Add a deterministic decision after extraction. The lab rule is: missing request ID, missing amount or negative amount goes to review; a complete non-negative request is ready for the next operation.

Read the structured fields and branch on those checks before any table write or external action. Keep the source case ID with both paths. Route node errors through **Try Catch** to a separate extraction-failed path. A failed call must not reuse a previous run's JSON.

Expected routes:

| Case | Route |
| --- | --- |
| complete | ready |
| missing | review |
| instruction | ready, only if extracted fields match source |
| business-rule | review |

For this practice, end both paths with a visible result or log containing the synthetic case ID and route. No external send is required. If the instruction case produces unsupported content, record the failure and stop it before the ready path; a valid schema alone is insufficient.

Completion: the four observed routes match, invalid-schema execution reaches the error path and no branch writes unvalidated data. Record provider/model and schema version so later comparisons use the same configuration.
