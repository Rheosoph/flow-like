"Received onboarding request." "Drafted a helpful reply and queued human review." Those two lines from the flight recorder didn't appear by magic — someone building this flow decided those moments were worth writing down, at that level, with those words. Next incident's thirty minutes are being budgeted right now, by the log lines you write today.

> **Predict first:** to be safe, should you log everything at Error level so nothing gets missed?

Hold that thought — the answer is at the end of the first section.

## Five levels, five jobs

The chips on the log panel — Debug, Info, Warning, Error, Fatal — aren't decoration. Severity is *information*, and each level has a job:

- **Debug** — evidence for future-you: intermediate values, branch decisions, payload shapes. "Reply drafted with confidence 0.42, routing to human review."
- **Info** — business milestones a teammate can read as a story. Both lines at the top of this lesson are Info: read them in order and you know what the run *did* without opening a single node.
- **Warning** — something unexpected happened and the run kept going. A retry that succeeded, an optional field that arrived empty.
- **Error** — the run cannot do its job. `Send Reply — body is null` belongs here.
- **Fatal** — the top of the severity scale, for the failures nothing recovers from.

Now the prediction: if you log everything at Error, the Error chip stops meaning anything. At 16:58 you'll click it hoping for one line and get two hundred, and you'll be reading noise on a deadline. Flattening severities deletes exactly the information that makes filtering work.

## Say what happened, name the value

Compare two versions of the same event:

> "Something went wrong."

> "Send Reply skipped: Body was null for request #4812 after review returned no draft."

The first line means "go spend twenty minutes finding out what I already knew." The second names the operation that failed, the value that was unexpected, and an ID to find the affected customer. When you write a log line, write it for the person reading it during an incident — name the operation, include the value, attach the identifier. Vague lines cost nothing today and everything on Friday.

## Choose the board's evidence level

Writing good lines is half the job. The other half is deciding how much evidence each run records — and that's a board setting.

@BoardVersions

That's the **Manage Board** dialog for the support board. Among its settings — name, description, stage, execution mode, version — sits a **Log Level** dropdown, currently set to **Debug**. This is the dial from the debug-loop infographic's footnote: log levels, Debug → Fatal, control how much evidence each run records. At Debug, runs record everything down to the chattiest lines. Raise the level and runs record progressively less.

The trade is straightforward. This board sits in the **Development** stage with Log Level Debug — maximum evidence while the flow is still being shaped. A busy production board often runs leaner, keeping the story lines and above. And when an incident hits a lean board and the log is thin? Open Manage Board, set Debug, reproduce the failure, and read the evidence you just enabled. The loop's step 1 exists precisely so you can replay a failure *after* turning the dial up.

**Watch out:** a thin log is not proof that nothing happened — it may be proof that nobody wrote it down, or that the board's level filtered it out. Check the dial before blaming the flow.

Recap:

- Five levels, five jobs — severity is information, so don't flatten it.
- A good line names the operation, the unexpected value, and an identifier.
- The board's Log Level in Manage Board decides how much evidence every run records; Debug for development, leaner for production, Debug again to hunt an incident.
