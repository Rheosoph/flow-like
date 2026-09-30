Use the intentional fault from the previous lesson. The log node reports the wrong value, but it did not create that value.

## Trace the producer

1. Find Print Info's **Message** input.
2. Follow its data wire to **To Upper Case**.
3. Follow that node's input to **Trim String**. Inspect the original value and trimmed value.
4. Compare each stage with the fixture: trimming is required; uppercasing is the incorrect operation.
5. Replace To Upper Case with **To Lower Case**. Reconnect the same input and Message destination. Change nothing else.
6. Run the original fixture. Expect `mixed case`.

Execution wires explain order. Data wires explain the origin of a value. A pure transformation may have no execution input, so following only execution wires misses the producer. If the producer sits inside a collapsed layer, open the layer and keep following the same data path.

In a failure such as a missing request body, start at the node and pin identified by the evidence, then work upstream until you find the first incorrect value. Replacing the node that reports the error may leave the cause untouched.

Record one sentence: “The log was wrong because To Upper Case supplied the value; changing the producer to To Lower Case satisfied the fixture.” Use equally specific explanations for real incidents.
