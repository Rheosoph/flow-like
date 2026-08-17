A ticket from mia@example.com just landed in the Customer Support Copilot. Everything you'd want while triaging it — her plan, her renewal date, whether she's already threatened to churn twice — lives in Orbit, your CRM, one HTTP request away. By the end of this lesson, the triage flow makes that request and reads the answer.

> **Predict first:** The API Call node has two execution outputs, Success and Error. Orbit answers your request with `404 Not Found` for an unknown email. Which pin fires?

The infographic below maps the whole course in one line: a trigger starts the flow, you build a request contract (URL, method, and body — then headers and authentication — then timeout and retry policy), you call the connected service, and you handle a typed response by validating the status, parsing the body, and storing or routing the result. This lesson walks the top row; the rest of the course fills in the hard parts.

@ApiIntegrationOverview

## 1 · Start from the trigger you already have

The pull begins where the agent does: the **Triage selected request** Quick Action. The Events course built surfaces like this one, so we won't rebuild it — just look at what it points to.

@QuickActionEvent

The event detail shows it Active, executing Local, type Quick Action, targeting the *Customer Support Automation* flow at version Latest through its Quick Action entry node. When an agent presses the button, execution enters that flow — and that's where our API call goes.

## 2 · Build the request

Open the node catalog and search the **Web/API** category. **Make Request** creates the request as a value: a Method dropdown (GET, POST, PUT, DELETE, PATCH — it defaults to GET) and a URL input, producing a Request struct on a data pin. For the pull: GET, `https://orbit.example/api/customers?email=mia@example.com`.

Nothing has been sent yet. A request in Flow-Like is data you assemble first, and the **Web/API/Request** nodes are its toolkit: **Set Header**, **Set Content Type**, **Set Accept**, **Set String Body**, **Set Struct Body**, **Set Form Body**, **Set Bearer Auth**. Each takes a Request, returns a modified Request. They're pure nodes — no execution pins, so they chain on the data path and cost nothing until something downstream actually needs the value.

## 3 · Send it

**API Call** is the node that talks to the network, so it lives on the execution path: an execution input, a Request input, and three outputs — **Success**, **Response**, and **Error**. Wire the triage entry's execution into it, connect your Request, run the Quick Action.

Here's the answer to the prediction: **Success fires only for 2xx status codes.** A 404 is a complete, well-formed HTTP exchange — and it still takes the Error path, with the Response pin fully populated so you can inspect what came back. Success and Error split *outcomes*, not *transport*. Only a genuinely failed exchange (no reachable server, for instance) errors the node itself.

## 4 · Read the answer

The Response is a struct, and **Web/API/Response** nodes take it apart. **Get Status Code**, **Is Success**, **Get Header**, and **Get Headers** are pure — probe them anywhere. **To Struct**, **To Text**, and **To Bytes** consume the body, so they sit on the execution path: wire Success into To Struct and Orbit's JSON becomes a struct whose fields — `plan`, `renewal_date` — flow onto pins your triage logic can use.

That's a working integration: trigger → Make Request → API Call → To Struct, and CRM data lands next to the ticket. Four nodes.

Try it yourself in any app: Make Request pointed at any JSON API you can reach, API Call, To Struct on the Success path, and a run to watch the response arrive.

One blemish before you get comfortable: that request works because Orbit's API token is currently pasted somewhere in the flow. Next lesson we make it disappear — without breaking the call.

## Recap

- A request is a value: Make Request creates it, pure Web/API/Request nodes refine it, and nothing is sent until API Call executes.
- Success means 2xx and nothing else; every other completed exchange takes the Error path with the Response still attached.
- Pure readers (Get Status Code, Is Success) probe the response anywhere; body readers (To Struct, To Text) run on the execution path.
