Add a fifth request: `Request R-105: Please correct my billing record. The amount is not provided.`

Expected fields: request ID `R-105`, category `billing`, amount `null`. Expected route: review. Run the request and record observed JSON and route, then rerun the complete fixture to prove a prior null did not become stale state.

Submit a five-row comparison of source, expected fields, observed fields and route. Include the invalid-schema failure result. These are manually inspected outputs; the questions do not run your model or inspect the board.
