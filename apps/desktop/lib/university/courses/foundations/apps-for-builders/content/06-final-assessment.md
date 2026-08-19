You've architected the Customer Support Copilot end to end: boundary, connectivity, surfaces, data, releases. Now you get to sit on the other side of the table. A neighboring team asks you to review their app design before launch — and it has exactly the kinds of problems you're now equipped to catch.

Keep @AppArchitecture in mind as your map: one App at the center, with Flows, Experiences, Data, Reuse, and Delivery hanging off it. Every question below asks whether this new app's parts form one operable product — and every answer combines concepts from at least two lessons.

## The scenario: Field Service Hub

Field technicians repair equipment on site. The team is building an **online App** with four surfaces:

- a **Page** where technicians review assigned jobs and upload photos;
- a **Quick Action** that records a device check using desktop-only input capabilities;
- a **nightly process** that summarizes completed work;
- an **authenticated API** that lets another internal system create jobs.

## The design you're reviewing

The current draft makes these choices:

1. All behavior lives in one large Flow with several unrelated entry nodes.
2. Job records are held in a Flow variable.
3. Shared repair manuals sit in one developer's User Storage.
4. The nightly Cron Event is Local, though no Desktop runner stays online overnight.
5. The API flow reads a provider secret saved only in one laptop's Runtime Variables.
6. Both production Events point to Latest.
7. Before launch, the team changed the App's version label to "1.0" — no numbered Flow version exists.

## Your job

For each risk, decide which boundary is drawn wrong and what the smallest correct fix is. The challenges below walk the review one risk at a time; expect each one to lean on two or more lessons — entry contracts and availability, storage scopes and credentials, versions and routes. A full explanation follows every answer, so commit to a decision before you check it.
