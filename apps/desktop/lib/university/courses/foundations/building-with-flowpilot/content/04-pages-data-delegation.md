Use a separate practice App for this optional class. Ask global FlowPilot:

```text
Create a page showing a single equipment-check status and a refresh button.
Use synthetic data. Include loading, empty, and error states. Also build the
workflow that provides the status and the Event that exposes the page.
```

Inspect three artifacts: the page, its backing workflow, and the Event. The UI specialist owns components and bindings; a separate workflow must supply behavior. A convincing preview with static data does not establish that the refresh button works.

Apply the reviewed artifacts, open the configured page, and use Refresh. **Check:** the action creates the expected run and updates the displayed value; an intentional empty result displays the empty state. If the provider cannot complete the whole request, reduce it to one artifact at a time.

For ordinary App use, FlowPilot prefers configured Events. It does not silently bypass a failed or declined Event by querying raw data. Ask explicitly for schema or table inspection when that is the task.

Continue with **Interfaces and Pages** for bindings and **Data in Flow-Like** for durable records.
