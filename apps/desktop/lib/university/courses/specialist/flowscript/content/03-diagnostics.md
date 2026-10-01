In Source normalizer's current anchored source, deliberately replace the `string` argument key in the trim call with `text`. Ask the workspace to compile or preview the edit.

**Expected:** the source cannot resolve that call shape against the declaration. Do not apply a failed edit. Read the diagnostic, inspect completion or the node declaration, and restore `string`.

Qualified names reduce ambiguity: `string::trim` identifies a namespace and operation. A call's return is its default output; use named outputs when a node has several. For unfamiliar nodes, consult the live catalog's declarations instead of guessing input or output names.

After repairing the key, change the literal to ` ash ` and run. Expect `ASH`.

**Check:** the invalid proposal was not treated as a verified board, and the repaired source produces the expected output. Keep the generated anchors around existing statements while making the correction.

[Calls and declarations](https://docs.flow-like.com/studio/flowscript/#calling-nodes)
