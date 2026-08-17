Friday, 16:58. The support app — the one that listens for requests, drafts replies with AI, and sends them after human review — has gone quiet. A customer wrote in forty minutes ago and nothing went out. You have a run log, thirty minutes, and a weekend on the line.

> **Predict first:** what's your first move — re-read every node on the board looking for something suspicious, or open the run that just failed?

Most people re-read the board. It feels productive, and it burns twenty of your thirty minutes on nodes that were never the problem. Here's what the run history already knows: every run leaves a trace, and the trace names the failing node. You never have to guess. You have to look.

## The loop

@DebugLoop

This infographic is the whole course in one picture: four steps — Reproduce, Isolate, Fix one thing, Verify — with a dashed arrow looping from the last step back to the first, and a safety-net note underneath about versions and log levels. Walk it with the Friday incident.

**1 · Reproduce.** Run the flow again with the same input that failed. Not a fresh test message you type from memory — the *same* input. On the card, the result comes back `run failed · 1.85 s`. That's not a defeat. A bug you can trigger on demand is a bug you can watch.

**2 · Isolate.** Open the run's log and find the node that failed. The card shows exactly the kind of line you're hunting: `✗ Send Reply — body is null`. One line, and your suspect list drops from "the whole board" to one node and one value.

**3 · Fix one thing.** Make the smallest change that explains the evidence — then stop. Not the smallest change plus a refactor plus two improvements you noticed on the way. One change.

**4 · Verify.** A fresh run, same input. Green? Run it once more to be sure. Still failing? Follow the dashed arrow back to step 1 — but as the caption on the arrow says, you loop again with one more fact than last time. Every pass shrinks the search space. Guessing doesn't.

## Why one thing at a time?

Because the loop is a measurement instrument. Change three things and the run goes green — which change fixed it? Did one of the other two quietly break something else? Change three things and it still fails — you've learned almost nothing, and you can't even cleanly undo. One change per pass keeps cause and effect attached to each other.

The bottom of the infographic names the two supports you'll pick up in later lessons: create a version before you fix, so you can roll back if the fix makes it worse, and use log levels (Debug → Fatal) to control how much evidence each run records. Both exist so that step 3 is never a leap of faith.

**Watch out:** pressure whispers "try everything at once." That's how a thirty-minute incident becomes a Monday incident. The loop is at its fastest precisely when you're stressed, because it replaces judgment calls with reads.

## Try it now

Open one of your own apps, open a flow you've run before, and find its run history in Studio. Don't fix anything — just confirm your past runs are sitting there, each with its own log. That's your flight recorder, and nobody had to switch it on. The next lesson teaches you to read it.

Recap:

- Every run leaves a trace that names the failing node — read it before touching the board.
- Reproduce → Isolate → Fix one thing → Verify; still failing means loop again with one more fact.
- One change per pass keeps cause and effect attached.
