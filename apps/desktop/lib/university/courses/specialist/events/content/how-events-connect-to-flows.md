Create a button that runs a flow without opening Studio. Use a scratch app and synthetic values throughout this course.

## Build

1. In Studio, add **Simple Event** and **Print Info**. Connect the event's Output execution pin to Print Info's execution input. Set the log message to `practice action received`.
2. Save the flow. Open the app's **Events** workspace and select **Create Event**.
3. Select this flow, its Simple Event node and **Quick Action**. Name it `Practice action`, save and activate it.
4. Press the action from the app sidebar. Open the flow's Runs panel and find the new run. Confirm the message appears once.

The **entry node** starts execution inside the flow. The **Event record** points to that node and configures how a caller reaches it. Deleting the Event removes the button; it does not delete the flow.

If the dialog offers no compatible node, save the flow and check its entry node first. If the action opens but the flow does not run, inspect the Event's selected flow, node and active state.

## Check your result

Press once more. You should see a second run with the same message. This exercise writes only a log, so repeating it has no external effect. Record the Event name and both run times. The question below checks the model; it does not inspect your app.
