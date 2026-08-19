New signups appear in Brightbeam's website tool. The CRM is a different system. Between them sits Priya, copy-pasting names and emails — the last manual chore on the list, and the one with the most embarrassing failure mode: a duplicate contact greeting a customer twice.

> **Predict first:** the outbound chain is Make Request → Set Header → API Call. Which of the three actually touches the network?

## 1 · The chore

When a signup happens, create the contact in the CRM — once. Glue, in both directions: the website *calls us* when a signup lands, and we *call out* to the CRM's API.

## 2 · The trigger

Inbound glue is an **API event**: one configured HTTP endpoint attached to a flow, with a path, a method, and an optional bearer auth token. It runs Local — served by the desktop app — or Remote, as a public URL on the hub. The website's "new signup" webhook points at it, and every delivery becomes a run.

One configuration choice matters more than the rest: **leave the auth token empty and the endpoint is public**. Fine for an afternoon of testing. In production, set the token and give it only to the caller — requests without it are refused with a 401.

## 3 · The flow

The outbound half, node by node:

1. **Make Request** — *builds* the request: method POST, the CRM's URL. Nothing is sent yet.
2. **Set Header** — attach the CRM's Authorization header. **Set Struct Body** shapes the contact payload.
3. **API Call** — the answer to the prediction: this is the node that performs the network call. Everything before it is preparation.
4. **Is Success** → **Branch** — never assume a 2xx. Check, then branch.
5. **To Struct** — parse the response body; store the returned CRM contact id with the signup, your durable proof of the outcome.

## 4 · Guardrails

- **Bounded retries.** The CRM has moods — an occasional 500 at busy hours. On failure: **Delay**, then try again, a *fixed* number of attempts. Unbounded retry loops turn one outage into a stampede. When the last attempt fails, stop and surface it — a failed run you can see beats a spinning one you can't.
- **An idempotent create.** Here's the recipe's dedupe key: the signup's email address. Check-before-create — ask the CRM for the email first, create only on miss. Now a retry, a re-delivered webhook, or Priya's nervous double-click all converge on one contact. Retrying a create *without* a dedupe key is how the embarrassing duplicate greeting happens.
- **Secrets stay out of the board.** The CRM token doesn't belong hardcoded in a pin where every screenshot ships it. Apps carry runtime variables for exactly this — and the API Integrations course goes deep on secrets, pagination, and rate limits. This recipe's rule is simply: the board holds logic, not credentials.

## 5 · Keep it

Endpoint details — the path, the flow it targets, its activation — live on the event, and event edits are staged:

@RouteConfiguration

That's an event from the support app open in Editing mode: name and description fields, a Route Path with its "must be unique" hint, dropdowns selecting the flow, version, and node — and along the bottom, the Editing mode bar with Discard and Save Changes. Nothing you type is live until Save; fat-fingered a path, press Discard. Config edits get the same safety your data does.

And keep verifying outcomes, not statuses: the weekly check for this recipe is counting signups against CRM contacts. Equal numbers, no duplicates — that's the green check that means something.

**Recap**

- Make Request builds, API Call sends, Is Success decides — in that order, every time.
- Retries are bounded and paired with a dedupe key, or they manufacture duplicates.
- Inbound endpoints get a bearer token in production; empty token means public.
