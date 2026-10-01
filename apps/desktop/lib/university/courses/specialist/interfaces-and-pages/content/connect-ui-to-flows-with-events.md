Use a separate Simple Event node named `Refresh queue` in the page's flow. This exercise sets a known value so you can inspect the entire action path before connecting a real data source.

## Wire the action

1. Connect the Refresh queue entry to **Data Update** (`a2ui_data_update`, UI/Data).
2. Copy the Page ID from Page Settings into Data Update's **Surface ID**. For this normal Page, that ID identifies its rendered surface; do not leave the node's default `main` unless it is your page's ID.
3. Set **Path** to `/status` and **Value** to the string `2 requests waiting`.
4. In the page builder, select Refresh and add a workflow action targeting the Refresh queue node. The builder supplies `workflow_event` and the selected `nodeId` with the owning app and board IDs. Use your actual nodes, not IDs from an example.
5. Open the Page through its configured UI Event and route. Press Refresh. Confirm the text changes and one run appears.

Navigation uses a separate `navigate_page` action. A button pointing at a route does not invoke your Refresh handler.

## Exercise the failure state

Change Data Update's value to `Could not load requests. Try again.` and invoke once. Restore the ready value and invoke again. This manually tests rendering, not network failure handling. When replacing the fixture with a request, route its actual error path to that error state and its success path to the data update. Set and clear the button's loading/disabled state on every terminal path.

If a run appears but the page stays unchanged, compare Surface ID and binding path. If no run appears, inspect the button action and selected entry node.
