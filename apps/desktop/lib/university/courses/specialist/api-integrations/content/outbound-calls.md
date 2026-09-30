Run a predictable API before using a business system. Download and unzip the files, then run `python3 mock_api.py` in that directory. Leave the terminal open. The service listens only on your machine and keeps its synthetic data in memory.

@PracticeFiles

## Build the request

Use a local scratch flow with **Simple Event**.

1. Add **Make Request**. Set Method to GET and URL to `http://127.0.0.1:8765/customers?email=learner@example.invalid`.
2. Connect its Request to **Set Bearer Auth**. Set Token to `practice-token`. This is a public fixture value, not a real credential.
3. Connect the resulting Request to **API Call**. Connect the entry execution output to API Call.
4. Connect Success to **To Struct**, passing the Response. Print the resulting struct.
5. On API Call's Error execution output, read **Get Status Code** from Response and print it.

Expected successful body: `{ "id": "C-001", "plan": "practice" }`. Change the email to `missing@example.invalid` and run again. Expect **404 on Error**, with a populated Response. A completed non-2xx HTTP exchange is different from a transport failure such as no listener at the address.

Make Request and the request modifiers construct a value. API Call performs the network operation. Response body readers such as To Struct run on the execution path; status/header readers inspect the Response value.

Stop the fixture with Ctrl-C after the course. A remote flow cannot reach this laptop's loopback address.
