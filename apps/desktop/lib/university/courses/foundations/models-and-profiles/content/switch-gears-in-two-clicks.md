Demo call in ten minutes. Your screen is still in train-mode: local drafting model, cozy theme. The customer expects the production gear: hosted heavyweight, work settings. This is not a settings-spelunking problem — it's two clicks in the upper-left corner.

## 1 · The switcher

That name at the top-left of the sidebar — the one that has read "Chatting · api.flow-like.com" through every screenshot so far — is a menu. Click it.

@SwitchAndEditProfiles

The Profiles menu lists three profiles: Chatting with an orange check (the active one) and ⌘1 beside it, Work with ⌘2, and Research with ⌘3 — the non-active rows each carry a small trash icon. Below the list sit two more entries: **Add profile** and **Edit profile**. Switching is one click on a row, or the ⌘-number without opening the menu at all. The active configuration changes on the spot; your apps don't move, duplicate, or vanish — lesson one's rule, now load-bearing.

## 2 · The machine room

**Edit profile** opens the full settings page for the active profile.

@ProfileSettings

For the Chatting profile, that page shows Profile Information — name, description, the current hub (api.flow-like.com), tags like "personal" and "assistant", interests like "conversation", "writing", "research" — next to a Profile Stats card counting apps, hubs, tags, and interests. Below: Execution Settings with a Max Context Size field (131072 here) and a GPU Mode toggle to enable GPU acceleration, Theme Settings, and Flow Settings with a Connection Mode picker for how flow wires are drawn. In short: everything that should flip *together* when your context changes, kept in one place, per profile.

## 3 · Build the two-gear setup

You now hold every piece of the course scenario. Assemble it:

1. **Add profile** → create your production gear (call it Work, like the screenshot does).
2. In Explore Models, stock it with the hosted heavyweights — the big-context, image-reading cards from the model-card lesson.
3. Keep the local gear lean: the on-device model downloaded *and* added to that profile, GPU Mode on.
4. Flip between gears from the upper-left — or with the ⌘-shortcuts printed right in the menu.

And here's the payoff for building on profiles instead of hard-coding: a flow doesn't have to name its model. **Find Model** picks one from the *active profile*, guided by preferences you compose with **Make Preferences** and **Set Preference Weight** — weighting cost, speed, reasoning, and friends. The same drafting flow that grabs the local model on the train selects the hosted heavyweight at the office, with zero edits — because the profile changed, not the flow.

That's the two-gear setup, complete. The final lesson hands it to a new teammate and sees what breaks.

Recap:

- Switch from the upper-left profile menu or its ⌘-shortcuts; add and edit profiles from the same menu.
- Per-profile settings include model assignments, theme, and execution options like GPU mode and max context size.
- Find Model plus preferences lets one flow ride whichever gear is active.
