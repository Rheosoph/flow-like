# Repository prose standard

These rules apply to user-facing prose across the repository: documentation, books, product
copy, tutorials, READMEs, release notes, examples, and comments that explain behavior. Code,
identifiers, protocol terms, verbatim quotations, and generated files keep their required
syntax.

The goal is useful writing with a recognizable human point of view. Do not optimize for an AI
detector. Detection is unreliable and can penalize non-native writers. Edit for the reader.

## Start with the reader

Before drafting, identify the primary reader and what the text should help that reader decide,
understand, or do. Infer this from the file and surrounding content when it is clear. If several
audiences are present, choose a primary path and layer optional detail after it.

- Executives need the consequence, business risk, fit with existing systems, and decision first.
- Developers need the system model, exact terms, constraints, integration points, and a working
  example.
- Operators need prerequisites, observable state, recovery steps, and the limits of the claim.
- End users need a goal-led path, expected result, and help when the result differs.

Do not make every paragraph serve every audience.

## Voice and structure

- Open with the concrete problem, outcome, or decision. Give the reader a reason to continue
  before introducing product vocabulary.
- Give each paragraph one job. Put its point near the start and remove sentences that only repeat
  it.
- Prefer named actors and concrete verbs. Say which component reads the file, who approves the
  release, or what fails.
- Define an unfamiliar term at first use. Use one term for one concept and preserve its
  capitalization. Do not rotate through decorative synonyms.
- Build interest with specific stakes, real examples, useful tension, and consequences. Avoid
  hype, vague urgency, and a string of slogans.
- Vary sentence length and openings when the subject calls for it. Avoid an unnaturally even
  cadence, repeated sentence frames, and clusters of dramatic fragments.
- Use headings and lists when they improve navigation. Ordinary prose does not need to be split
  into many small sections, and a rhetorical list does not become clearer because it has three or
  five items.
- Prefer a short word when it is equally precise. Keep established technical terms even when a
  synonym sounds more elegant.

## Patterns to remove

- Do not use em dashes in prose. Rewrite with a period, comma, colon, or a new sentence.
- Avoid the templates “not X, but Y,” “it is not X; it is Y,” and repeated negative-to-positive
  contrasts. State the accurate claim directly. Use a contrast only when a real misconception
  must be corrected.
- Avoid forced groups of three or five, especially adjective chains, rhetorical questions,
  successive fragments, and slogan-like conclusions.
- Remove stock openings and transitions such as “In today’s fast-paced world,” “Moreover,”
  “Furthermore,” “Ultimately,” and “Taken together.” Use a transition only when it names a real
  logical relationship.
- Cut throat-clearing, recaps of the preceding paragraph, obvious conclusions, and caveats that
  are repeated in several sections.
- Do not invent quotations, customer speech, or composite dialogue. Quote only exact, attributable
  language. Otherwise paraphrase.
- Do not claim that something is simple, seamless, robust, secure, scalable, or production-ready
  without showing the relevant fact or boundary.

## Technical prose

- Put the mental model before implementation detail. A reader should know what an object is and
  why it matters before seeing its internal name.
- Keep procedures chronological. State prerequisites before commands and the expected result
  after them.
- Pair important constraints with their practical consequence. “The Event is pinned to version
  4” matters because later drafts cannot change the live endpoint.
- Separate current behavior, preview behavior, and plans. Link claims to code, measurements, or a
  named source where appropriate.
- Explain integration with existing systems when presenting a platform or architecture. Most
  readers are extending an estate, not starting from an empty project.

## Editing pass

Read the draft aloud, then check:

1. Does the opening serve the primary reader?
2. Is every product term defined before it carries explanatory weight?
3. Can any paragraph or sentence disappear without losing meaning?
4. Are actors, constraints, and consequences specific?
5. Did any banned template, em dash, fake quotation, forced list, or vague claim survive?

For long-form prose, compare word count before and after. A shorter draft is better only when it
keeps the facts, voice, and necessary context.
