Every weekday at 8:15, Dana logs into the Kestrel Components supplier portal — a web app that looks exactly the way it did in 2009 — and checks the status of roughly forty open purchase orders, one search at a time, copying each status into a spreadsheet. Forty-five minutes, every morning, before her real work starts. Kestrel has no API, and their roadmap says "maybe next year." You're going to give Dana her mornings back by automating the portal itself. By the end of this lesson you'll know when UI automation is the right tool, when it isn't, and the four-stage strategy every flow in this course follows.

> **Predict first:** before you drag a single node, what's the first question to ask about the Kestrel portal — which surface clicks fastest, or something else entirely?

## The two questions

**Is there an API?** If Kestrel exposed one, you'd call it and this course would be over. An API is a contract built for machines; a UI is a picture built for people. That's why the docs are blunt about it: when the target system exposes a reliable API, use it — the API Integrations course owns that path. UI automation (RPA, if you like the classic name) is for systems whose interface is the *only* interface.

**Are you allowed?** Working credentials aren't permission. Automating a system you don't own touches its terms of service and your agreement with the vendor, so get the answer in writing before you build. Dana's team asked; Kestrel confirmed that automated read-only status checks are within the agreement. That one sentence will outlast every retry policy you ever write — file it somewhere safe.

The portal passes both: no API, explicit authorization. Now you're in RPA territory, and you need a strategy sturdier than "click where Dana clicks."

## Surface → Target → Interact → Verify

@DesktopAutomationOverview

That's the whole method on one card: **choose a surface** (Browser, Computer, or Vision), **resolve the target** deterministically (browser selectors, accessibility elements, image templates, coordinates), **perform the action** (click and type, mouse and keyboard, extract or capture), then **prove the outcome** (assert the resulting state, save a checkpoint, capture diagnostic evidence). The strip along the bottom names the support layers you'll meet later: RPA controls for timeouts, bounded retries, and recovery; Selector and Fingerprint nodes for ranked deterministic fallbacks; and optional LLM assistance that observes, plans, or heals — and then gets validated. Every lesson from here walks some stretch of those four stages.

## Choose a surface

Flow-Like's Automation catalog gives you three ways to touch an interface:

- **Browser** nodes drive web pages through selectors and page state. The Kestrel portal's lane.
- **Computer** nodes drive native desktop apps through accessibility elements, windows, and direct mouse and keyboard input.
- **Vision** nodes match image templates and pixel colors — for interfaces that expose neither selectors nor accessibility metadata.

The rule worth memorizing: **prefer the most deterministic surface available.** A selector beats a screen coordinate for a web page; an accessibility element beats an image match for a native control. Vision and LLM nodes are deliberate fallbacks, not defaults — reaching for them first trades reliability for convenience.

One constraint to plan around: automation runs where the interface lives. Browser and computer automation need a compatible local execution environment, and desktop interactions also depend on the operating system, the active session, and permissions. Dana's flow will run on the ops workstation, where the portal login and the desktop tools actually are. (Firing it every morning on a schedule is trigger territory — the Events course covers that.)

## Try it — two minutes

Open any board, right-click the canvas, and type "browser" into the Actions menu's search box. Scroll the Browser family: opening and closing, navigation, clicking, typing, waiting, extraction. No need to place anything yet — just know where the tools live before the next lesson picks them up.

## Recap

- API first: UI automation is for systems that offer nothing better — and it's authorized in writing before it's built.
- Every automation here follows Surface → Target → Interact → Verify.
- Prefer the most deterministic surface: Browser for the portal, Computer for native apps, Vision as the fallback.
