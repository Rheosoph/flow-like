export const enBots: Record<string, string> = {
	"bots.meta.title": "Bots: AI teammates that work while you don't | Flow-Like",
	"bots.meta.description":
		"Flow-Like bots are AI teammates with their own name, abilities and host. Wake them with messages, schedules, email and webhooks, run them in the cloud, on your desktop or on your own hardware, and stay in charge of everything they do.",
	"bots.meta.keywords":
		"AI bots, AI agents, AI teammates, autonomous agents, scheduled AI, email agent, webhook agent, local AI agent, self-hosted AI agent, multi-agent rooms, Flow-Like",

	"bots.hero.eyebrow": "FLOW-LIKE BOTS",
	"bots.hero.title.l1": "A crew that works",
	"bots.hero.title.em": "while you don't.",
	"bots.hero.body":
		"Bots are AI teammates with a name, a face and a job. Give each one the abilities it needs, pick where it runs, and wake it with a message, a schedule, an email or a webhook. It works with your rights, and anything that matters asks you first.",
	"bots.hero.cta.primary": "Build your crew",
	"bots.hero.cta.secondary": "Read the docs",
	"bots.hero.status.scout": "Reading 14 sources",
	"bots.hero.status.post": "3 replies drafted",
	"bots.hero.status.byte": "Chart ready",
	"bots.hero.status.flux": "Building your app",
	"bots.hero.status.loop": "All quiet since 06:00",

	"bots.jobs.eyebrow": "JOBS",
	"bots.jobs.title": "Hire for the job.",
	"bots.jobs.body":
		"Say what the bot should do in one sentence. Flow-Like picks the closest preset and sets up its look, abilities, host and data label. Change anything you like, or start from scratch.",
	"bots.jobs.example":
		"“Every morning, look up what our competitors announced and send me a short digest.”",

	"bots.wake.eyebrow": "ALWAYS ON",
	"bots.wake.title.l1": "It doesn't wait",
	"bots.wake.title.l2": "to be asked.",
	"bots.wake.body":
		"Schedules come due, mail arrives, webhooks fire, flows ask, someone mentions it in a room. Every trigger lands in your chat with the bot, so you follow up right where it happened.",
	"bots.wake.pause": "Pause",
	"bots.wake.play": "Play",
	"bots.wake.chatWith": "Your chat with {name}",
	"bots.wake.inRoom": "Room: {name}",
	"bots.wake.e1.trigger": "Scheduled: Morning digest",
	"bots.wake.e1.reply":
		"Four competitor launches overnight. Two matter for us: Northwind cut its prices by 20%, and Fabrikam shipped offline mode. Links are in the digest.",
	"bots.wake.e2.trigger": "Email from lena@acme.com: Invoice #4471",
	"bots.wake.e2.reply":
		"I matched the invoice to the March order. The amounts agree, so I drafted a confirmation.",
	"bots.wake.e2.approval": "Post wants to email lena@acme.com",
	"bots.wake.e3.trigger": "Webhook: deploy.failed",
	"bots.wake.e3.reply":
		"billing-api failed its health check twice after the 10:12 deploy. I paused the rollout and sent you a push.",
	"bots.wake.e4.trigger": "@Byte how did signups do last week?",
	"bots.wake.e4.reply": "Up 12% week over week. Tuesday did most of the work.",
	"bots.wake.e5.trigger": "Event: From a flow",
	"bots.wake.e5.reply":
		"Two runs failed today, both timeouts in the CSV import. I attached the logs.",
	"bots.wake.e6.trigger": "Scheduled: Competitor price watch",
	"bots.wake.e6.reply": "Nothing changed tonight, so Scout stayed quiet.",
	"bots.wake.deny": "Deny",
	"bots.wake.once": "Allow once",
	"bots.wake.always": "Always allow",
	"bots.wake.kind.schedule": "Schedule",
	"bots.wake.kind.email": "Email",
	"bots.wake.kind.webhook": "Webhook",
	"bots.wake.kind.room": "Room",
	"bots.wake.kind.flow": "Flow",
	"bots.wake.kind.monitor": "Monitor",

	"bots.hosts.eyebrow": "ANYWHERE",
	"bots.hosts.title": "Runs where your data lives.",
	"bots.hosts.body":
		"Every bot lives on one host. Put it in the cloud to work around the clock, in your desktop app to use local models and your screen, or on a device you control for unattended work on your own hardware.",

	"bots.hosts.where": "Where the bot runs",
	"bots.hosts.unavailable": "Not in the cloud",
	"bots.hosts.answers": "Answers",
	"bots.hosts.models": "Models",
	"bots.hosts.bestFor": "Best for",
	"bots.hosts.server.label": "Cloud",
	"bots.hosts.server.tagline":
		"Around the clock, whether your computer is on or not.",
	"bots.hosts.server.answers": "Within seconds, day and night.",
	"bots.hosts.server.models":
		"Hosted models in your plan, or your own provider keys.",
	"bots.hosts.server.bestFor":
		"Digests, inbox triage, webhooks and watching your apps.",
	"bots.hosts.desktop.label": "Desktop app",
	"bots.hosts.desktop.tagline":
		"On your computer, with your screen, browser and terminal.",
	"bots.hosts.desktop.answers":
		"While the app is running, on your computer or phone.",
	"bots.hosts.desktop.models":
		"Every model in your profile, including local ones.",
	"bots.hosts.desktop.bestFor":
		"Private data, local models and hands-on work at your desk.",
	"bots.hosts.device.label": "Your hardware",
	"bots.hosts.device.tagline":
		"A server or mini PC you manage, running the device agent.",
	"bots.hosts.device.answers": "Whenever the device agent runs, unattended.",
	"bots.hosts.device.models":
		"Hosted models through Flow-Like's model service.",
	"bots.hosts.device.bestFor":
		"Browser automation and shell work on hardware you control.",

	"bots.charge.eyebrow": "IN CHARGE",
	"bots.charge.title.l1": "Powerful.",
	"bots.charge.title.l2": "Never unsupervised.",
	"bots.charge.body":
		"Bots act for you, so they never get more than you have, and they stop for anything with consequences.",
	"bots.charge.rights.title": "Never more than you",
	"bots.charge.rights.body":
		"In every app a bot works with your role, narrowed by the limits you set for it and for each room. Lose access and your bots lose it too.",
	"bots.charge.asks.title": "Asks before it acts",
	"bots.charge.asks.body":
		"Mailing someone new, calling a new site, running a command: each stops for Allow once, Always allow or Deny. Always allow stays narrow, and mail or web pages can never grant permission.",
	"bots.charge.secrets.title": "Secrets it can't see",
	"bots.charge.secrets.body":
		"API keys are write-only. The model only sees a placeholder, the host swaps in the value at the last moment, and anything that echoes it back gets scrubbed.",
	"bots.charge.data.title": "Data that stays put",
	"bots.charge.data.body":
		"Label what a bot may handle. Once it reads something more sensitive, its mail, web and connector tools refuse for the rest of the turn.",

	"bots.rooms.eyebrow": "TOGETHER",
	"bots.rooms.title": "Rooms for people and bots.",
	"bots.rooms.body":
		"Bring colleagues and bots into one conversation. Bots read along and know when to stay quiet; an @mention always gets an answer.",
	"bots.rooms.point1":
		"Invite other people's bots. Their owners decide what they may do in your room.",
	"bots.rooms.point2":
		"What a bot learns in a room stays there, unless you choose to share it.",
	"bots.rooms.point3":
		"Your bots can ask each other for help, with your permission, and every exchange is on record.",
	"bots.rooms.point4":
		"Jump on a call with up to 8 people, right from the room.",
	"bots.rooms.chat.room": "Q4 launch",
	"bots.rooms.chat.mara": "@Byte how did signups do last week?",
	"bots.rooms.chat.byte": "Up 12% week over week, mostly from Tuesday.",
	"bots.rooms.chat.scout":
		"That's the day the newsletter went out. Open rate was 41%.",
	"bots.rooms.chat.you":
		"Nice. @Post draft a thank-you for the newsletter team?",

	"bots.memory.eyebrow": "MEMORY",
	"bots.memory.title": "Remembers, with receipts.",
	"bots.memory.body":
		"Bots keep the facts that matter, each tied to the message it came from. Review them on a timeline or a map and forget any one of them. Content from the web or mail can't quietly plant memories.",
	"bots.memory.fact1": "Wants the digest before 8:00",
	"bots.memory.fact1.quote":
		"“can you send it before 8? I read it on the train”",
	"bots.memory.fact1.kind": "Preference",
	"bots.memory.fact2": "Q4 launch moved to November 28",
	"bots.memory.fact2.quote": "“we're pushing the launch to the 28th”",
	"bots.memory.fact2.kind": "Decision",
	"bots.memory.fact3": "Works from the Berlin office on Tuesdays",
	"bots.memory.fact3.quote": "“Tuesdays I'm in Berlin”",
	"bots.memory.fact3.kind": "Profile",

	"bots.flux.eyebrow": "MEET FLUX",
	"bots.flux.title": "One of them builds apps.",
	"bots.flux.body":
		"Flux is FlowPilot as a member of your crew. Describe the app you wish existed and Flux builds its interface, workflows and data. Mention it in a room or wake it from a schedule; when your desktop is online, it gets its full build toolkit.",
	"bots.flux.link": "What Flux can build",

	"bots.abilities.eyebrow": "ABILITIES",
	"bots.abilities.title": "26 abilities. All off until you say so.",
	"bots.abilities.body":
		"Switch on exactly what a job needs. Screen, browser, shell and end-to-end encryption need a host you control.",
	"bots.abilities.local": "Desktop or device",
	"bots.abilities.link": "Explore every ability",

	"bots.cta.title": "Build your crew.",
	"bots.cta.body":
		"Assistant and Flux are waiting in the desktop app, no account needed. Add your first bot in a minute.",
	"bots.cta.download": "Download Flow-Like",
	"bots.cta.web": "Open the web app",
	"bots.cta.docs": "Read the bot docs",
};
