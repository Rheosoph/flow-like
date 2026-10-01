Build a form that accepts a synthetic request. Prerequisite: you can add a node and edit its pins.

## Create the payload path

1. Add a **Generic Event** to a scratch flow. Add a String output named `subject` and an Integer output named `quantity`.
2. Connect its execution output to **Print Info**. Connect `subject` to the log message. Save the flow.
3. Create an Event targeting this node. Select **Generic Form**, give it the route `/practice-request`, save and activate it.
4. Open the route in the app. Submit `Replacement cable` and `2`.
5. Inspect the resulting run. Confirm the subject reaches the log. Inspect the entry output to check that quantity is a number.

Use the fixture values below if you want a record of expected inputs.

@PracticeFiles

A route maps a path to a UI Event. The Event selects the surface, such as a form, chat or Page. A Page by itself does not own an address. Paths start with `/` and must be unique within the app. Use `/` for the app's default surface when you are ready to configure it.

## Check a failure

Try to reuse `/practice-request` for a second UI Event. Keep the existing owner and cancel the conflicting edit. Then submit a blank subject to the original form. Observe what the interface permits: validation in your flow is still required before any real write or send.

Backend Events such as Cron and API use their own trigger configuration; a UI route is not their endpoint. Interfaces and Pages covers Page actions and data binding.
