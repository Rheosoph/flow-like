Somewhere past the fourth tunnel, the train Wi-Fi gives up for good. Every hosted model in the catalog just became a very pretty picture. Gemma 4B Local didn't notice — its 2.0 GiB of weights sit on your disk, and your machine does the thinking.

> **Predict first:** offline drafting sounds like pure upside. What are you trading away for it? At least three honest answers are printed on the model card itself.

## 1 · Why local at all

Three reasons, in the order support teams usually care about them:

**Privacy.** An on-device model processes your prompt on your device. The half-written apology, the pasted account details — none of it travels to a provider. When the policy says "customer data must not leave the laptop", on-device isn't a preference, it's the requirement.

**Offline.** No connection, no problem. The train, the plane, the client site with the hostile guest Wi-Fi — your drafting gear keeps working.

**Cost shape.** A hosted model can require a configured provider or account, and usage runs through it. A local model spends your disk and your hardware instead — resources you already paid for.

## 2 · What it costs you

Now read Gemma's card with cold eyes: `Text → Text · 33K ctx · On-device`, download size 2.0 GiB.

- **Disk.** Model weights are real files — 2.0 GiB here, and local models use disk space in general.
- **Hardware.** Local models execute on supported local hardware. Your laptop does work a provider's datacenter would otherwise do.
- **Headroom.** 33K of context against Claude Sonnet 4's 200K or Gemini 2.5 Flash's 1.0M. Plenty for drafting a reply; cramped for "consider this entire 300-page manual".

A compact local model is a sharp junior drafter, not a heavyweight reasoner. The two-gear plan embraces that honestly: local for drafts on the go, hosted for production polish. You're not choosing a winner — you're staffing two different shifts.

## 3 · Download, then assign — two separate acts

@AddingModelToProfile

Open the three-dot menu on the Gemma 4B Local card and you get exactly two actions: **Download (2.0 GiB)** and **Add to Profile**. They are not the same thing, and confusing them produces this course's most common "it's broken":

- **Download** copies the model files onto the *current device*. Nothing more.
- **Add to Profile** puts the model into your toolkit, so the profile's AI features and compatible nodes in Studio can actually use it.

The catalog keeps you honest on both counts: it shows which profiles already include a model and whether a download or update is still in progress.

> **Watch out:** a downloaded model that was never added to a profile won't be offered anywhere. If a Studio node's model list comes up empty-handed, check the profile before you re-download anything.

Your offline gear is now real: files on disk, model in the profile, and a drafting setup that treats dead Wi-Fi as scenery.

Recap:

- Local buys privacy, offline work, and a friendlier cost shape — at the price of disk, hardware, and context headroom.
- Download = files on this device. Add to Profile = usable in flows. Two acts, always both.
- The catalog shows per-profile membership and download progress.
