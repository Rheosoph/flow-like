Use the tested Normalize Label Flow from Thinking in Flows, or a Simple Event → Print Info Flow with a known message. Record its expected output first.

Enable [Developer Mode](https://docs.flow-like.com/start/developer-mode/) so the App navigation shows **Flow Templates**.

1. Create a numbered Flow version from the tested draft using **Manage Board → Create Version**.
2. Open the App's **Flow Templates** workspace and choose **Create Template**.
3. Name it **Practice label**, choose your source Flow and its numbered version, then create it.
4. Use that template to create another Flow. Configure any dependencies the template does not carry, then run it.

**Expected:** the new Flow produces the recorded output. This verifies the captured graph in a second instance.

Choosing **Latest** at template creation also captures a snapshot at that moment. It does not subscribe to future draft edits. To distribute a later fix, add a newer source snapshot as another template version and test the version consumers choose.

A template carries a graph, not the source App's runtime credentials, Events, Pages, files, or tables. A Flow using those resources needs them configured in its destination.

[Template creation and versions](https://docs.flow-like.com/apps/templates/)
