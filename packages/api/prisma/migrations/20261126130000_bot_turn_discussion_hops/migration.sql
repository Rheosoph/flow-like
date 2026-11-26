-- A bot turn records how long a person allowed its discussion between bots to run, so the later turns of that chain keep the budget.
ALTER TABLE "BotTurn" ADD COLUMN "discussionHops" INTEGER;
