Give the practice Flow a button users can invoke outside Studio.

1. In your practice App create a Flow named **Record check**.
2. Add Simple Event and Print Info. Connect Simple Event **Output** (`exec_out`) to Print Info **Input** (`exec_in`).
3. Add a String variable **Equipment Label**, default `TEST-01`, and enable **Exposed**.
4. Add **Get Equipment Label** and connect its value to Print Info **Message**. Set **On Screen?** to true.
5. In the App's Events workspace create a **Quick Action** targeting this Flow and its Simple Event. Use Latest for this private exercise and activate the Event.
6. Invoke the action from the App's use interface. Enter `TEST-02` in its exposed variable field.

**Expected:** the resulting run logs `TEST-02`. A successful Studio run with the default `TEST-01` would not establish that the action passed its input correctly.

The entry-node family constrains the available interfaces. Simple Event supports Quick Action; other interfaces use different contracts. Follow **Events** for API, Cron, chat, and route setup. Follow **Interfaces and Pages** when a form or dashboard needs a custom layout.

[Event contracts](https://docs.flow-like.com/apps/events/)
