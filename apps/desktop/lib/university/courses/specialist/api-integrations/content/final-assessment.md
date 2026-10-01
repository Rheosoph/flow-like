Use the local mock to collect three pieces of evidence: a valid customer response, an unknown customer and a rejected credential. Then submit one authenticated local webhook and one unauthorized attempt.

Record status, invoked branch and intended outcome for each. Stop the mock service and confirm that a transport failure is distinguishable from an HTTP 404. Restore your harmless fixture flow before finishing.

If you completed the partial-failure class, also record the note count before and after retry. These are manual observations; the quiz does not inspect your machine or claim that the integration is deployed.
