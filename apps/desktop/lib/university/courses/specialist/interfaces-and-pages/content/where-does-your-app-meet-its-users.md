You will build a queue page with a status label and a Refresh button. Use a scratch app with a flow that has a Simple Event. Complete Events first if creating or routing an Event is unfamiliar.

A **Page** belongs to a flow and provides a full interface. A **Widget** is an app-owned reusable block placed on pages. **Chat UI** is a configurable conversational surface. Choose a Page when people need to scan records and take explicit actions; choose chat when the interaction is a conversation.

The visual builder and Dev Mode edit the same A2UI component model. Components have stable IDs, containers reference child IDs, and displayed values can be literals or bindings. Keep the queue heading literal. Bind the changing status.

## Start the page

Enable [Developer Mode](https://docs.flow-like.com/start/developer-mode/) to access the editing workspaces. Open your flow and select **Explorer → UI → Create Page** (+). Name the page `Practice queue`, create it, then open it from the Explorer. Add a Column with a Text heading, a second Text component and a Button. Give the second Text the ID `queue-status`; label the button `Refresh`.

Preview the page. You should be able to find the heading, status and one clear action without scrolling sideways. The next lessons add data and behavior.

In the app's **Events** workspace, create an Event with this Page as its target and `/practice-queue` as its unique route. Save and activate it, then open that route in the app. Keep this Event for testing the Refresh action later in the course.
