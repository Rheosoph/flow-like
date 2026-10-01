Turn a free-form request into typed fields and route it through deterministic checks. You need a configured tool-capable model and basic Flow branching. Use synthetic inputs throughout.

Download the output contract and four cases:

@RequestSchema
@ExtractionCases

The schema requires `request_id`, `category` and `amount_eur`. Missing identifiers/amounts become `null`; category is one of three declared values. Extra fields are rejected. Required means the key must exist, not that its value must be invented.

Read each case and predict its JSON before invoking a model. An example JSON object can infer a schema, but an explicit schema makes nullability and allowed categories reviewable.

The final negative amount is valid schema data. This lab's business policy sends negative adjustments for review, demonstrating why shape validation and business validation are separate checks.

Completion: explain every field and its missing-value behavior. Deterministic parsing remains appropriate when the source already has a reliable JSON contract. See [structured extraction](https://docs.flow-like.com/topics/genai/extraction/).
