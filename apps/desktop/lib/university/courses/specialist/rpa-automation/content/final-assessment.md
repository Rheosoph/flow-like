Monday morning, you present the finished automation plan. On the table: one week of dry-run findings, a written authorization, and a workstation. Everything below is real input. Every challenge that follows pulls at least two lessons together — and nothing in this file answers them for you.

## The goal

Every weekday before 8:00: check the status of roughly forty open purchase orders on the Kestrel Components supplier portal, and export the day's shipping labels from the Kestrel label client. Output: the tracking spreadsheet, filled and verified, plus evidence for each run. (Scheduling the morning trigger is Events-course machinery — here you're judged on the automation itself.)

## The constraints

- Kestrel's written authorization covers **automated read-only status checks**. Nothing else.
- **No API today.** Kestrel has announced a status API beta for next quarter.
- The label client has **no API and no web version**, and its accessibility tree is nearly empty.
- The flow runs on the ops workstation: two monitors, display scaling recently changed to 125%.
- Every run must leave evidence a colleague can audit.

## The dry-run week

| Day | Finding |
|-----|---------|
| Monday | Manual baseline: 47 minutes, zero errors. Dana is accurate — just busy. |
| Tuesday | Coordinate-click prototype: the portal displayed a maintenance banner, the flow copied the wrong column into the spreadsheet, and the run reported green. Caught by hand at 8:40. |
| Wednesday | Label export failed: IT changed display scaling from 100% to 125%, and the Export-button template stopped matching. |
| Thursday | Portal slow: the fixed five-second delay elapsed before the table rendered, the extraction came back empty, and the run reported green. |
| Friday | An experimental branch clicked Acknowledge on delayed POs; after a timeout it retried, and one PO was acknowledged twice. Kestrel noticed. |
| Saturday | A diagnostic ticket went out with a full-screen capture attached — including a customer's name and delivery address. |

## Your job

Seven calls to make. The pass isn't reciting node names — it's making the decisions that keep Monday at 8:15 boring, authorized, and on the record.
