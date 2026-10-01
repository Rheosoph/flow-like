This recipe creates a report file. Sending it is a separate effect and is deliberately left for a reviewed extension.

1. Upload `weekly-metrics.json` to scratch app Storage.
2. Use **Storage Dir** and a child path to the fixture, then **Read to String**. Print the text once to verify the input.
3. For this fixed fixture, set a **Write String** node's content to the supplied expected report text. Resolve its destination under `practice-reports/report-2026-W33.txt` using a typed Path.
4. Execute manually and read the file back. Run again. Expect one path with the same content.
5. Create a Cron Event on the flow's Simple Event. Use one future occurrence for testing, read its next-run preview and observe the actual result.

For the recurring version, choose `0 9 * * 1` with `Europe/Berlin`. A local schedule needs Desktop running. Use a supported hosted scheduler for unattended delivery and test it there. Hybrid may register both delivery paths, so do not treat it as deduplication.

A report key identifies the reporting period. In a real flow, derive the period from the intended reporting window, including catch-up behavior, rather than the retry's wall-clock time.

If you later add Send Mail, use a supported send contract and a separate delivery record/reconciliation policy. Overwriting the weekly file does not prevent a second email. Inspect both outcomes before marking a period complete.
