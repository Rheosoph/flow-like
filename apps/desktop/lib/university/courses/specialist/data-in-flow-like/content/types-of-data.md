Your practice app receives a request and imports three ticket records. Decide where each value belongs before writing it.

| Value | Home | Lifetime |
| --- | --- | --- |
| Current input text | Typed pin value | This run |
| Uploaded source file | App or User Storage | Until deliberately removed |
| Ticket ID and status | Native table | Across runs |
| One query result | Query session, unless saved | This run |
| Customer-to-ticket relationship | Ontology definition over tables | With its model |

Use **Storage** for shared app files and **User Storage** for files private to one user. A derived file needs the same access consideration as its source. Folder names such as `private/` do not establish permission boundaries.

Download the synthetic fixture. It contains an original import, a changed record and expected counts.

@PracticeFiles

Inspect `tickets.json`: `ticket_id` identifies a business record. Keep that ID when a status changes. A timestamp of the import run identifies the attempt, not the ticket.

Create a scratch app or use a disposable practice table. You will import only the fixture, then repeat the same import and change one row.
