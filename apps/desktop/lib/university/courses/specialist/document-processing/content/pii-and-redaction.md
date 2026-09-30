The fixture includes a synthetic signer name and email. Mask them before creating shared copies or indexes.

1. Pass extracted text into **PII Mask (Regex)** with email detection enabled. Inspect Masked Text, Detection Count and Detections.
2. Confirm the email from `expected.json` is absent from Masked Text.
3. If names require masking, pass the result through **PII Mask (AI)** with an approved configured model, or route the document to a manual review step when one is unavailable.
4. Inspect the signer name in the final masked result. Detection counts alone cannot prove it was removed.
5. Build any shared summary or search input from the masked text. Inspect those derived outputs too.

Regex detects known shapes. Contextual model detection can miss names or other indirect identifiers. A mask removes only what it detects, so compare against the known fixture fields and retain a review outcome.

Store source IDs, method/configuration and inspection result as evidence. Do not copy the sensitive values into ordinary logs. The original and the shared copy can require different access scopes.

This lab checks the fixture, not general legal compliance. For another document mix, choose and test the required field coverage before release.
