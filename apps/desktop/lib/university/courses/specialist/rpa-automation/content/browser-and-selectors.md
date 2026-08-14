Your first prototype logged into the portal with recorded clicks: move to (412, 380), click, type the password, click again at (412, 440). Monday it worked. Tuesday, Kestrel pushed a maintenance banner across the top of every page, the whole layout slid down eighty pixels, and your flow typed Dana's password into the "Remember me" checkbox. No error. The run went green.

> **Predict first:** the login button moved eighty pixels. Which still finds it — the recorded position, or a description of the element itself?

## 1 · Surface: open it, and always close it

A browser automation starts with **Open Browser**, which connects to a WebDriver server and opens a fresh browser session, and **New Page**, which gives you the tab you'll work in. **Go To URL** takes you to the portal's login page. The mirror image matters just as much: **Close Page** and **Close Browser** belong on the cleanup path — the one that runs on success *and* failure. A flow that only closes the browser when everything went well leaks sessions on every bad day, and bad days are why you're here.

## 2 · Target: describe the element, not the pixel

A coordinate describes a place on the screen. A selector describes the element itself — "the input named `username`", "the button with id `login-submit`". When Tuesday's banner shoved the layout down, the recorded coordinates kept pointing at the same place while every element left it. A selector would have followed the element wherever the layout put it.

Not all selectors age equally, though. Prefer ids and stable attributes over positional chains like "the fourth cell of the third row" — the fewer assumptions a selector makes about layout, the longer it lives. For targets you'll rely on daily, the Selector family (**Build Selector**, **Create Selector Set**, **Rank Selectors**, **Get Best Selector**) lets one flow carry several ways to find the same element, ranked — you'll wire those into deliberate fallbacks in the robustness lesson.

Finding all these nodes is a search away:

@NodeCatalog

In that shot, a board's Actions menu is open with "mail" typed into the search box, and the mail family — Send Email, Watch Inbox, Parse Mailbox and friends — surfaces instantly. Type "browser" instead and the Browser nodes appear the same way.

## 3 · Interact: wait, then act

The portal renders its login form a beat after the page loads, so the first interaction node isn't a clicker at all — it's **Wait For Selector**, which waits until an element matching your selector exists in the DOM. Then **Type Text** fills the username and password fields, **Click Element** submits, **Select Option** picks "Open orders" from the status filter dropdown, and **Press Key** covers the odd Enter or Tab the portal insists on.

The sequence for any form, worth walking once by hand:

1. **Wait For Selector** on the field you're about to touch.
2. **Type Text** (or **Select Option**) to fill it.
3. **Click Element** to submit.
4. Wait for the *result* — **Wait For Selector** on the status table, or **Wait For Network Idle** when no single element marks completion.
5. Read and verify before moving on.

## 4 · Verify: read it, prove it, keep it

**Get Text** pulls the status cell for each purchase order; **Get Attribute** and **Get HTML** cover the cases where the value hides in markup rather than visible text. **Take Screenshot** — or **Screenshot Element** for just the table — keeps evidence of what the portal actually showed. If the portal offers a CSV export, downloads have their own choreography: **Set Download Directory** *before* anything downloads, **Trigger Download** to click the export link, **Wait For Download** until the file lands.

Logging in forty times a week is also worth avoiding: **Save Cookies** at the end of a run and **Load Cookies** at the start of the next keeps the session alive between mornings. Treat that cookie file like a credential, because it is one.

> **Watch out:** a green run proves the nodes executed — not that they did the right thing. Tuesday's flow "worked" while typing a password into a checkbox. Verification is part of the flow, not an afterthought; lesson four makes it systematic.

## Recap

- Selectors track elements; coordinates track pixels — and pixels lie the day the layout changes.
- Wait For Selector or Wait For Network Idle is your readiness signal; a fixed delay is a guess.
- Open Browser and New Page start the session; Close Page and Close Browser end it on every path.
