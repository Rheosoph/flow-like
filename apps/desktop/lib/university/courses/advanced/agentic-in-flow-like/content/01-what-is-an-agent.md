Your ops team already runs one automation, and you're about to look at it: a request comes in, a model drafts a reply, a human approves, the reply goes out. Nobody calls it an agent. Then Priya from ops posts a new request: "Why is checkout latency spiking since yesterday's deploy?" Try to sketch that flow. Does it search the runbooks first, check service status, or read the deploy log? You can't decide until you've seen the first result — and neither can a fixed board. That gap is what this course closes. Across nine lessons you'll design an ops assistant that answers from runbooks, researches the public web when it has to, and reads live status over MCP — without ever handing the model the keys.

## 1 · The flow you already have

@AgentNodeAnatomy

This is the support board in Studio. The **Incoming Support Request** event fires (it's selected here, so its little toolbar floats above it), execution travels along the white wires through **Prepare Support Reply** and **Human Review** to **Send Reply**, and the dashed pink wires underneath carry the request text and the drafted reply. The tinted comments — *1 · Listen for requests*, *2 · Draft with AI*, *3 · Approve and send* — label the stages, and the grey sticky notes explain the two middle nodes. A model writes the draft, but it never chooses what happens next. Same steps, same order, every run. For this job, that's exactly right.

## 2 · The request that breaks it

Priya's latency question has no fixed sequence. The useful next step depends on what the last step revealed: if status shows the payment provider degraded, stop and say so. If not, search the runbooks for the deploy checklist. If those don't mention latency, check the provider's public status page. Something has to *choose*, mid-run.

That something is an agent: a configured model, operating instructions, and a bounded set of approved tools. The model may pick which tool to call, read the result, and adapt its next step. Everything else — who may do what, how long the run may last, what gets recorded — stays with the Flow. An agent isn't a chat model with a long prompt, and it isn't an autonomous principal. It's a decision-maker inside a cage you build.

## 3 · The loop and its cage

@BoundedAgentLoop

The infographic shows that cage precisely. Inside the boundary, the agent cycles **Plan** (pick the next step) → **Act** (call one tool) → **Observe** (read the result) → **Verify** (done, or loop again?), with *max loops · max cost* and *stop early on failure* printed at the center. The boundary itself carries three labels — **Permissions**, **Step & budget limits**, **Audit log** — and there's exactly one exit: a **Verified result** with evidence and logs attached. A check that fails stops the loop early; a bounded agent can't run away.

Notice where each control lives. The prompt can *ask* the model to stay inside tenant scope — that's guidance. Only a tool that derives tenant scope from trusted identity, and a Flow that enforces turn and time budgets, make the boundary real.

## 4 · Agent or fixed flow?

Ask two questions before you reach for autonomy. Does the task require choosing among several approved tools based on intermediate results? Does the request need interpretation before it maps to an action? Two nos means build a fixed flow: "rotate the gateway key following runbook 12" is a known sequence and should stay one. Autonomy buys adaptivity and costs you variability, model spend, new failure paths, and a bigger permission surface.

When the answer is yes, write a short operating contract before you open Studio: the outcome in one sentence, the approved read and write tools listed separately, the budgets (turns, time, output), and what the user sees on no result or partial failure. The next lesson turns that contract into nodes.

> **Watch out:** the most common agent bug isn't in a node — it's *prompt-as-permission*, trusting a sentence in the system prompt to do a tool's authorization job. You'll meet it again in every module of this course.

## Recap

- A fixed flow runs known steps; an agent chooses its next bounded action from what it just observed.
- The model plans and picks tools; permissions, budgets, and audit stay in the Flow.
- Write the operating contract first, then build.
