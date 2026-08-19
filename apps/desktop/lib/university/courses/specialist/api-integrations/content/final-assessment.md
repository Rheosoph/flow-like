This is the synthesis exam. Everything you need is in the scenario below — the challenges do the asking, and each one draws on at least two lessons.

## The project

The support team is shipping the full two-way integration between the Customer Support Copilot and Orbit CRM. Sign-off requires every requirement met and every dry-run finding closed.

## Requirements

- **R1 — Context pull.** When an agent triages a ticket, the flow fetches the customer's account record from Orbit's API using the ticket's email address and attaches plan and renewal data to the ticket.
- **R2 — Resolution push.** When a ticket is resolved, the flow POSTs one resolution note to Orbit's API. Exactly one — Orbit deduplicates nothing on its side.
- **R3 — Plan-change webhook.** Orbit notifies the support app the moment a customer's plan changes. Notifications must land around the clock, with no desktop machine involved, and only Orbit may trigger the flow.
- **R4 — Reporting service.** Orbit's backend queries ticket summaries through an authenticated, multi-endpoint service. The credential must be scoped to the app and survive any single teammate leaving the company.
- **R5 — Credentials on laptops.** Agents authenticate the triage pull with a CRM token configured on their own machines. Nothing credential-shaped may appear in the flow definition or travel with the app when it syncs.

## Constraints from Orbit's API documentation

- Rate limit: 60 requests per minute; exceeding it returns `429 Too Many Requests`.
- Occasional `503` blips during Orbit deploys, typically gone within seconds.
- Unknown email addresses return `404 Not Found`.
- Bearer-token authentication on every endpoint.
- Customer lists are paginated, 50 records per page.

## Dry-run findings

- **F1.** The webhook flow was switched to remote execution for the 2 a.m. test. It failed immediately: the CRM token variable — configured and working on the developer's laptop — resolved to nothing on the server.
- **F2.** During a slow Orbit deploy, one resolved ticket produced two identical resolution notes on the customer's record in Orbit.
- **F3.** Overnight, a teammate saved a draft edit that renamed a field in the summaries flow's output. Orbit's parser broke at 6 a.m. Nobody had touched the event configuration.
- **F4.** A hand-crafted request to the webhook endpoint — sent from a personal machine, with a made-up customer email — created a plan-change record in the app.

Work through the challenges. Where a finding and a requirement collide, the requirement wins.
