# Board ⇄ AST ⇄ Text fixture

This Chapter 14 fixture is the canonical final state of the Incident Desk rule used to explain
parsing, rendering, and minimal reconciliation. It derives `customerFacing` from the existing
report inside the Function instead of changing that Function's established signature.

The fixture is parser-tested with the invariant `render(parse(source)) == source`. The chapter's
one-literal-to-one-`UpdateNodePin` claim is covered by the core reconciler's anchored literal-edit
regression test. Catalog-aware structural command counts deliberately remain preview-dependent.
