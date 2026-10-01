Add evidence around the normalization flow without exposing private payloads.

Use **Print Info** for a completed operation, **Log Warning** for an unexpected condition that can continue, and **Log Error** for a failed operation. Use debug-level evidence while investigating values or branches; record only what the investigation needs.

For this synthetic exercise, log the fixture ID and result. For real customer text, prefer an identifier, lengths, selected branch and duration. Do not copy secrets, authorization headers or full documents into logs.

## Inspect filtering

1. In Manage Board, inspect **Log Level**.
2. Run the synthetic fixture with Info recording enabled and find its line.
3. Filter the log view to Error only. The Info line disappears from this view; the transformation did not stop running.
4. Restore the view. Compare the stored recording level with the viewer filter: they control different stages.

For a slow flow, compare node durations before changing its design. A high token count may explain one model call's cost; it does not prove another node is cheap. Record a baseline, change one cause and compare the same workload.
