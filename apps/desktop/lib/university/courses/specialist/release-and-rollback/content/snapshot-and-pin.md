Create a scratch flow with **Simple Event → Print Info**, Message `release A`. Create and activate a Quick Action targeting its Latest draft. Invoke it and inspect the result.

@PracticeFiles

1. In Studio, open **Manage Board → Create Version**. Choose an appropriate version type and create the snapshot.
2. Inspect the numbered snapshot and record its exact version. The editable draft continues separately; do not infer the published number from the draft label alone.
3. Edit the Quick Action and select that numbered flow version. Save the Event.
4. Change the draft message to `release B` and save it.
5. Invoke the pinned Quick Action. Expect `release A`. A Studio run of the changed draft should print `release B`.

Latest follows the editable draft. A numbered snapshot gives callers a stable target until the Event is deliberately repointed. An app's descriptive version field is not a flow snapshot.

Keep both observed runs and their executed versions. This is your rollback target and comparison input for the next step.
