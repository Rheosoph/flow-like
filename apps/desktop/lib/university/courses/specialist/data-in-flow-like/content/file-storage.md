Use the fixture from the previous lesson. Keep the source file so you can explain the result later.

1. Open the scratch app's **Storage** and upload `tickets.json` into a `practice-imports` folder.
2. In a flow, add **Storage Dir** and use **Child** from the catalog to resolve this file under the returned root. Pass typed Path values between storage nodes.
3. Add **Hash File**, connect the file Path and execute it from a Simple Event. Connect Hash to **Print Info**. Record the resulting hash and filename.
4. Repeat without changing the file. The hash should match. Upload `tickets-updated.json` and hash it separately; its contents differ, so its hash should differ.

A filename is a label. A source identifier plus content hash tells you which version was processed. Store the source identity, version/hash and processing outcome alongside durable imports when provenance matters.

Use User Storage for an exercise with per-user input. A file in app Storage is shared according to app access; adding a folder called `restricted` does not make it private. Keep secrets out of either file store.

For this lab, do not move the source as a completion signal. In a real import, record success or archive only after writes are confirmed. If parsing fails, preserve the source and the error location so a corrected attempt can be traced.

Verify by reading the file back from the app. The expected result is the three original records, without renamed IDs or missing fields.
