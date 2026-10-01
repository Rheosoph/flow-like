Create one record per agreement from the extracted fixture. Use this schema:

```json
{
  "source_id": "agreement-practice-001",
  "source_version": "fixture-v1",
  "agreement_id": "PRACTICE-001",
  "renewal_days": 30,
  "monthly_fee_eur": 125,
  "source_pages": [1],
  "extraction_method": "deterministic",
  "review_status": "checked"
}
```

Use `expected.json` to verify the numbers against the original page. Do not replace uncertain values with plausible guesses. Preserve the source path, hash/version and page locations even if you also store a summary.

For document discovery, **RAKE Keywords** and **YAKE Keywords** provide deterministic key phrases. **AI Keywords** can use a context instruction when the intended tags depend on meaning. Test the output against an explicit search need rather than requiring every algorithm.

**Extract Content Sections** and **Summarize Document** can add a navigable summary. If you use them, verify the summary's facts and page references before delivering it. A concise summary that changes a renewal period is a failed result. Keep the original as the authority.

For this exercise, write the verified record to a scratch table with the stable source ID, using the upsert pattern from Storage and Tables. The next lesson controls which text may be shared.
