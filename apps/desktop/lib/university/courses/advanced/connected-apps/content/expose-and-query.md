In the producer's **Data Studio → Model**, create an ontology over `parts`. Define a `Part` object with unique ID `part_id`, display property `name`, and projected properties `part_id`, `name`, `stock`. Keep the source rows in their table.

1. Open **Sharing** and turn on **Expose to connected projects** for this ontology.
2. In the consumer's Sharing tab, discover contracts from the active producer connection and install this contract.
3. Confirm its object type appears in the consumer's Sources/Explore view. Inspect the two fixture parts.
4. Add **Query Remote Ontology Objects** to a consumer Flow using the installed contract and Part binding. Request a bounded result and inspect it.

Expected result: P-101/Blue bracket/12 and P-102/Small plate/0. Queries execute against the producer, with the connection's authorization. Installation provides a sanitized contract and generated bindings; it does not copy all source data or the producer's board implementations into the consumer.

If discovery is empty, check exposure, active connection direction and producer data-read permission before changing the ontology. If installation fails, check the consumer user's data-write permission.

Completion: record the installed contract version and the two observed objects. See [ontology setup](https://docs.flow-like.com/topics/ontology/overview/) for mapping controls.
