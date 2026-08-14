Three internal teams run the support kit now, and the partner org keeps asking for "that triage thing". Time to put the app in the store. Friday at 16:50, Priya opens Visibility Status and reaches for **Public**.

> **Predict first:** She flips the switch. Is the app live in the store on Monday morning?

## 1 · The gate: review, not a switch

No. Moving from Prototype to **Public Request** or **Public** submits the app for central review, which takes one to three business days — you're notified once it completes. The store isn't an upload button; it's a reviewed shelf. Plan the lead time into any launch date, and remember the two public rungs differ only in access mode: **Public** lets everyone join directly, **Public Request** makes visitors request access first (the store badges it "Request access"). You can switch between the two later without losing your listing. Retreating from a public rung back to Prototype removes the app from public visibility — and that transition goes through review as well.

## 2 · Polish the listing before you submit

Reviewers and store visitors both read your metadata, so make the Details tab earn its place: a name that says what the app does, a description that says who it's for. Two release-metadata habits from the versioning world carry over here:

- The app's free-form **Version** field is a label for people browsing — editing it does *not* snapshot the app, its flows, storage, or events. Treat it as the name of a tested collection of flow and interface versions.
- Record what changed in the app changelog, so an update is a story instead of a surprise.

The **Publication** page in the app's navigation (under Insights) is your mission control for the process: it tracks publication review history and the compliance panels that go with a listing. Offline projects never appear here — an app that isn't online is never listed, so no review is required until you bring it online.

## 3 · After launch: ship updates on purpose

Once the app is listed, strangers depend on it — which changes how you edit. Two disciplines keep them safe:

- **Pin what the store serves.** Production-facing events should target numbered flow versions, not Latest, so your Tuesday draft experiments never reach store users mid-edit. (The Events course covers pinning in depth.) When an improvement is ready: create a new flow version, test it, repoint deliberately, and keep the old version available as the rollback path.
- **Revisit the fork policy.** A public app can be forked by anyone allowed to — before launch, re-decide what a stranger's fork should contain. Client data in the database? Almost certainly "tables only" or "no database".

Then close each release the boring way: bump the Version label, write the changelog line, done.

> **Watch out:** A listing is a promise. The review gate checks the app once — keeping drafts away from store users is *your* ongoing job, and version pinning is how you do it.

## Recap

- Public and Public Request go through central review (1–3 business days) — plan launches with lead time.
- The app Version field and changelog are release metadata for humans; they snapshot nothing.
- After launch: pin store-facing events to numbered versions and re-check the fork policy.
