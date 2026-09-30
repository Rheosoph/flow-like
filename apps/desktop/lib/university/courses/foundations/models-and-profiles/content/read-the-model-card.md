Read the current model card and provider configuration before choosing a model. Names and catalogue examples change; capability requirements remain useful.

| Requirement | What to check |
| --- | --- |
| Read an image | The configured model accepts image input |
| Process a long document | Context capacity, output budget, and any file-processing limits |
| Produce vectors for search | Embedding output and model/configuration consistency |
| Run without a connection | On-device execution and all required local files |
| Use tools in FlowPilot | Tool support in the chosen provider path |

Choose a second candidate for the same `cedar` prompt. Record capability, execution location, setup requirement, and observed response. Do not assume the more expensive model is better for this small task.

**Check:** each candidate satisfies the actual input/output contract. If a card and provider documentation disagree, verify the configured endpoint before depending on the capability. A model family name alone does not establish what that configuration accepts.

[Models in workflows](https://docs.flow-like.com/topics/genai/models/)
