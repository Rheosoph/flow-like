Create one status card definition and place it on two practice pages.

1. Open the app's **Widgets** workspace and create `Queue status card`.
2. In its builder, add a Card containing a Text heading and Text status. Give each a stable ID.
3. Expose the heading's content as a configurable property named `Card title`.
4. Place the widget on two scratch pages. Change the shared definition and confirm both instances reflect it.

For different instance titles, use Dev Mode or FlowPilot to set the instance's exposed-property values. The current Page Builder does not mount the per-instance inspector, so inserting a widget initially creates empty overrides. Inspect the actual property/instance IDs in the generated JSON before editing. Do not invent a widget ID or assume a visual control exists.

A reusable button should emit a named Widget event. Define the action in **Settings → Events**, use a `widget_event` action with that `actionId`, then bind it to the selected page workflow in the instance's `actionBindings`. This keeps the definition independent of one page's flow. Use a scratch copy while learning the JSON binding format.

Check the card with an empty status, a long heading and a narrow container. Keep one definition; use instance properties for intended differences.
