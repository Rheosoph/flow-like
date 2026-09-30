Use the existing tickets table and a small `practice_customers` table containing one row each for C-001 and C-002. Remove the intentional duplicate from the SQL fixture before using it here.

An **ontology** maps authoritative tables to business objects and links. Creating the overlay does not copy all source rows into another database.

1. In **Data Studio → Model**, create a practice ontology.
2. Map `practice_customers.customer_id` as the Customer identity and its name as a display property.
3. Map `practice_tickets.ticket_id` as the Ticket identity. Expose status and customer_id.
4. Define the Customer-to-Ticket relationship using `customer_id`.
5. In **Explore**, open C-001. Expect tickets T-001 and T-002. Open C-002 and expect T-003.
6. Change T-002's status through the core upsert flow. Reopen the object and verify the authoritative record is reflected.

Null or duplicate identity values make a model ambiguous. A missing link target must be handled explicitly rather than silently assigned to another customer.

Define a governed action when the interface should offer a controlled operation, such as assigning a ticket. Specify its permitted actor, required input and outcome before connecting a write. Overlay sharing is not a grant of access to every underlying row.

For a schema change, inspect dependent object properties, links, saved queries and actions before renaming columns. Dropping an overlay removes definitions; dropping its source table removes data.
