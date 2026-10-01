Test contract change and access revocation separately.

Add a string `location` column to the producer table with values `A1` and `B2`, and include it in the Part object's projected properties. Save the producer contract. In the consumer, use Discover/Refresh to inspect **Update available**, then refresh the installed copy. Updates are not pushed automatically.

Query again. Expected locations: A1 for P-101 and B2 for P-102. Confirm the updated binding's schema rather than assuming an old installation refreshed itself.

Turn off producer exposure. Start a fresh consumer run and query the installed contract. Expect authorization failure. Keeping a local contract copy does not preserve access after revocation; authorization may be cached only within a run, so test with a new run.

If you later expose a governed action, enable sharing for that action as well. **Invoke Remote Ontology Action** executes the producer's pinned implementation and parameter contract. The consumer's local object sheet does not invoke remote actions. Treat writing actions as a separate reviewed extension to this read-only lab.

Completion: retain one refreshed result and one denied fresh-run result. You can uninstall a local contract even when its connection is inactive, allowing cleanup after revocation.
