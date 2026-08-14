Quarter-end at Brightbeam. The Monday checklist is gone — five recipes run it now, and Priya spends her mornings on work that needs a human. This morning she hands you the runbook and this week's incident list, and asks you to close it out.

## The runbook

| Recipe | Trigger | Act | Dedupe |
| --- | --- | --- | --- |
| Inbox triage | 15-min sweep | Move mail, create drafts | `UNSEEN` filter — handled mail exits it |
| Monday report | Cron `0 9 * * 1`, `Europe/Berlin` | Write report file, send mail | File named after the ISO week |
| Status bot | Telegram on a Chat Event | Send Message reply | Stateless answers |
| Drop-folder watcher | 15-min sweep | Process CSVs, write results | Diff manifest, committed after success |
| Signup glue | Inbound API event → CRM API call | Create CRM contact | Email as dedupe key, check-before-create |

## This week's incidents

1. **The missing report.** Monday's metrics mail never arrived. The Runs list shows no entry since last Monday. The ops laptop spent the weekend in a bag.
2. **The double batch.** Someone switched the watcher's schedule to Hybrid "for reliability." Friday's partner batch was processed twice.
3. **The office-hours bot.** The status bot answers 9-to-6 and goes silent at night. Its token is valid and the flow is unchanged.
4. **The déjà-vu drafts.** Since a teammate edited the triage flow "to be safe," the same three mails get fresh reply drafts every fifteen minutes.
5. **The night visitors.** The signup endpoint logged 240 runs overnight from a caller nobody recognizes. The endpoint was set up during testing and never touched since.
6. **The triple greeting.** During a flaky CRM afternoon, three duplicate contacts appeared — each one greeted the same customer separately.
7. **The promotion.** The watcher has passed every hand-triggered dry-run. Priya wants it moved onto its 15-minute schedule this week and asks what evidence should exist first.

Each item below combines at least two recipes' lessons. No walkthroughs this time — the runbook and the symptoms are everything you need.
