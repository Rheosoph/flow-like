Create or select a second practice Profile with a different assigned model. Open the Profile menu, switch, and inspect the picker again.

Run the same one-word prompt with each model. Record Profile, selected model, response, and any setup failure. Keep inputs constant so the comparison has one clear purpose.

A workflow can select from the active Profile through **Find Model** and preference nodes. A node configured to use a particular model is a different selection path: switching Profile does not rewrite its hard-coded configuration. When a workflow uses an unexpected provider, inspect how the model was selected.

Use this troubleshooting order:

1. Confirm the active Profile and selected provider/model.
2. Check Profile membership and, for local models, completed download.
3. Check provider authentication or hardware/load errors.
4. Confirm that the requested capability is supported.

**Check:** both trials are accounted for, including a precise error if one configuration cannot run. Do not label a model operational solely because it appears in the catalogue.

[Profiles](https://docs.flow-like.com/start/profiles/)
