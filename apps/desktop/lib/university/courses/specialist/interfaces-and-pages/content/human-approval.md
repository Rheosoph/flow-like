Build an approval interface before connecting it to a consequential action. This lab records only a synthetic decision; it does not implement a durable approval service.

Use `review-request.json` from Practice Files. Display its request ID, proposed message and destination on a Page. Add **Approve** and **Reject** buttons and a status Text component. The reviewer should see the exact proposed action before choosing.

## Build two handlers

1. Add two Simple Event nodes, `Approve practice request` and `Reject practice request`.
2. In each handler, use **Print Info** to record the synthetic request ID and decision. Do not log the proposed message when adapting this to real content.
3. Follow with **Data Update**, using the Page ID and `/status`. Set `Approved for practice` or `Rejected for practice` respectively.
4. Bind each button's workflow action to its corresponding entry node.
5. Open through the route. Test both decisions, including keyboard-only use. Record which run each button produces.

## Test the boundary

Double-click Approve, refresh the page and invoke again. The log-only prototype may record repeated decisions. A production approval needs an authoritative pending record, permitted reviewer identity, expiry, and an atomic transition that accepts only one decision. A disabled button alone does not enforce any of these rules. Reject and expired cases must never reach a send/write handler.

Write the server-side acceptance rule for this fixture: request `review-001`, pending state, expected revision `1`, permitted reviewer, and current time before expiry. Connect a real action only after the application enforces that rule and records who approved it. The API partial-failure lab covers duplicate requests.
