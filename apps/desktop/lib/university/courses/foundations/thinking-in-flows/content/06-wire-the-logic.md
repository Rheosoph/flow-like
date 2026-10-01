Create a Flow named **Normalize Label**. Its job is to turn surrounding whitespace and lowercase letters into a clean uppercase label.

| Node | Configure or connect |
| --- | --- |
| Simple Event (`events_simple`) | Output → Print Info Input |
| Trim String (`string_trim`) | String = `  cedar  ` |
| To Upper Case (`string_to_upper`) | String ← Trimmed String |
| Print Info (`log_info`) | Message ← Uppercase String; On Screen? = false |

The execution path contains the event and logger. The two transformations are pure data dependencies. Do not add execution wires to them.

Run the event: expect `CEDAR`. Then use these synthetic test cases. Enter each input into Trim String's String pin and inspect the matching run.

@NormalizerCases

| Input | Expected message |
| --- | --- |
| `  cedar  ` | `CEDAR` |
| ` Birch ` | `BIRCH` |
| `two words` | `TWO WORDS` |
| three spaces | an empty message |

An empty message is a valid result here. A production label form may reject it; **Control Flow** teaches that decision.

You can create compatible connections by dragging from a pin into empty canvas and choosing a suggested node. Always confirm the node and input you selected. The table above is the contract for this build.
