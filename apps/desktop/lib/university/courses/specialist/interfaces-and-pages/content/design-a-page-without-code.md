Give the queue status a data binding. Download the fixture states, which contain no real customer data.

@PracticeFiles

## Bind the status

1. In the page builder, select `queue-status`.
2. In Dev Mode, set this Text component's `content` property to `{ "path": "/status", "defaultValue": "Not loaded" }`.
3. Set the page's `dataModel` to `[{ "path": "/status", "value": "2 requests waiting" }]`. Keep the rest of the builder-generated document intact.
4. Preview. The status should read `2 requests waiting`.
5. Change the data entry to `No requests waiting`. The same component should change without editing its content binding.

Use the Inspector for layout and typography. Semantic classes such as `bg-background`, `text-foreground` and `text-muted-foreground` follow the app theme.

## Test the state table

Apply each value from `states.json` to `/status` and inspect the result:

| State | Expected text | Action |
| --- | --- | --- |
| Initial | Not loaded | Refresh available |
| Loading | Loading requests | Prevent another in-flight Refresh |
| Empty | No requests waiting | Refresh available |
| Ready | 2 requests waiting | Refresh available |
| Error | Could not load requests. Try again. | Retry available |

These are manual previews. The next lesson wires the ready state; a real request must set loading/error states from its execution paths.

Test a narrow preview, a long status message, light and dark themes, and keyboard focus. The status must remain readable and the button must have a visible label and focus indicator. Do not rely on green or red alone to communicate the result.
