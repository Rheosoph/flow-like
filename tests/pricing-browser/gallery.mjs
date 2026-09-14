import { readFile, readdir, writeFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import sharp from "sharp";

const output = resolve(
	process.env.SCREENSHOT_OUTPUT ?? "output/pricing-ui-screenshots",
);
const capturedOn = new Date().toISOString().slice(0, 10);
const captureDate = new Intl.DateTimeFormat("en-GB", {
	day: "numeric",
	month: "long",
	year: "numeric",
	timeZone: "UTC",
}).format(new Date(capturedOn));
const app = JSON.parse(
	await readFile(join(output, "app-manifest.json"), "utf8"),
);
const known = new Map(app.captures.map((item) => [item.file, item]));
const files = (await readdir(output)).filter(
	(file) =>
		file.endsWith(".png") &&
		!["preview.png", "gallery-preview.png"].includes(file),
);
const human = (value) =>
	value
		.replace(/\.png$/, "")
		.replaceAll("-", " ")
		.replace(/^./, (value) => value.toUpperCase());
const captures = await Promise.all(
	files.map(async (file) => {
		const info = known.get(file) ?? {
			file,
			title: human(file),
			group: file.startsWith("admin-")
				? "Admin"
				: file.startsWith("website-")
					? "Website"
					: "Quota warnings",
		};
		const { width, height } = await sharp(join(output, file)).metadata();
		return { ...info, width, height };
	}),
);
const groups = [
	"Plans",
	"Quota warnings",
	"Upgrade dialogs",
	"Upgrade edge cases",
	"Usage",
	"Usage details",
	"Mobile",
	"Website",
	"Admin",
];
captures.sort(
	(a, b) =>
		groups.indexOf(a.group) - groups.indexOf(b.group) ||
		a.file.localeCompare(b.file),
);
const escape = (text) =>
	String(text).replace(
		/[&<>"']/g,
		(character) =>
			({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
				character
			],
	);
const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Flow-Like pricing screenshots</title><style>
:root{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;background:#0c0e12;color:#f4f4f5}*{box-sizing:border-box}body{margin:0}header,main{max-width:1480px;margin:auto;padding:32px}header{padding-top:48px;padding-bottom:24px}h1{font-size:clamp(30px,4vw,48px);letter-spacing:-.04em;margin:8px 0 16px}p{line-height:1.65;color:#a6abb6;max-width:920px}.eyebrow{font-size:12px;color:#ff764f;letter-spacing:.13em;text-transform:uppercase}nav{display:flex;gap:8px;flex-wrap:wrap;margin-top:24px}button,input,a{font:inherit}button,.download{border:1px solid #343841;border-radius:8px;background:#171a20;color:inherit;padding:9px 13px;cursor:pointer;text-decoration:none}button.active{border-color:#ff6534;color:#ff9f80;background:#302019}input{border:1px solid #343841;background:#13151a;color:inherit;border-radius:8px;padding:11px 14px;width:min(100%,430px);margin:0 0 20px}.count{color:#a6abb6;margin-left:12px;font-size:14px}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:20px}article{border:1px solid #2b2e36;border-radius:12px;overflow:hidden;background:#14171c}article a.shot{display:block;background:#1c2028;height:240px;overflow:hidden}article img{display:block;width:100%;height:100%;object-fit:contain;object-position:top center;transition:transform .2s}article a.shot:hover img{transform:scale(1.025)}article .meta{padding:18px}h2{font-size:16px;margin:6px 0 10px;line-height:1.4}.tag{font-size:11px;color:#ff9f80;text-transform:uppercase;letter-spacing:.08em}.dimensions{font-size:12px;color:#8f97a6}article .download{font-size:12px;padding:5px 8px;display:inline-block;margin-top:12px}footer{max-width:1480px;margin:30px auto;padding:32px;color:#8f97a6;font-size:13px}dialog{width:94vw;height:94vh;max-width:none;max-height:none;background:#0c0e12;color:#fff;border:1px solid #343841;border-radius:12px;padding:0}dialog::backdrop{background:#000c}.dialogbar{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:12px 18px;border-bottom:1px solid #343841}.dialogbar div{display:flex;gap:8px}.viewer{height:calc(100% - 64px);overflow:auto;text-align:center;padding:20px}.viewer img{max-width:100%;height:auto}.viewer.zoom img{max-width:none}.empty{color:#aaa;padding:30px}button:focus-visible,a:focus-visible,input:focus-visible{outline:2px solid #ff764f;outline-offset:3px}@media(max-width:640px){header,main{padding:20px}.grid{grid-template-columns:1fr}.dialogbar{flex-wrap:wrap}.viewer{padding:8px}}
</style></head><body><header><div class="eyebrow">Flow-Like · Pricing update</div><h1>Every pricing screen, ready to review.</h1><p>${captures.length} screenshots of the implemented components. Plan amounts come from the current repository configuration. Account, app, model and usage data are illustrative fixtures. The shared Web/Studio components and website pricing sections are rendered locally; these are not captures of a production deployment.</p><p>Includes monthly and annual tiers, 75% / 90% / 100% warnings, all quota dialogs, shared billing limits, usage breakdowns, pending costs, free alternatives and Max administration. Includes the refreshed plan and usage tabs, compact upgrade recommendations, expandable comparisons and operation detail panels. Admin screenshots are retained from the earlier capture.</p><nav aria-label="Screenshot categories"><button class="active" data-group="All">All</button>${groups.map((group) => `<button data-group="${escape(group)}">${escape(group)}</button>`).join("")}</nav></header><main><label><input id="search" aria-label="Search screenshots" placeholder="Search screens, states or themes…"></label><span class="count" aria-live="polite"></span><div class="grid">${captures.map((item) => `<article data-group="${escape(item.group)}" data-search="${escape(`${item.title} ${item.file}`.toLowerCase())}"><a class="shot" href="${escape(item.file)}" data-title="${escape(item.title)}"><img src="${escape(item.file)}" alt="${escape(item.title)}" loading="lazy"></a><div class="meta"><span class="tag">${escape(item.group)}</span><h2>${escape(item.title)}</h2><div class="dimensions">${item.width.toLocaleString()} × ${item.height.toLocaleString()} px · PNG</div><a class="download" href="${escape(item.file)}" download>Download original</a></div></article>`).join("")}</div><p class="empty" hidden>No screenshots match this filter.</p></main><footer>App screenshots captured from the local implementation on ${captureDate}; website and admin captures include the preceding day. Screenshot PNGs are unretouched browser captures. Sample provider rates are illustrative, not live pricing.</footer><dialog><div class="dialogbar"><strong id="image-title"></strong><div><button id="zoom">Actual size</button><a class="download" id="original" download>Download PNG</a><button id="close">Close</button></div></div><div class="viewer"><img alt=""></div></dialog><script>
const articles=[...document.querySelectorAll('article')],count=document.querySelector('.count'),search=document.querySelector('#search');let group='All';function filter(){let n=0;for(const article of articles){article.hidden=(group!=='All'&&article.dataset.group!==group)||!article.dataset.search.includes(search.value.toLowerCase());if(!article.hidden)n++;}count.textContent=n+' screenshots';document.querySelector('.empty').hidden=n>0;}for(const button of document.querySelectorAll('nav button'))button.onclick=()=>{group=button.dataset.group;for(const other of document.querySelectorAll('nav button'))other.classList.toggle('active',other===button);filter();};search.oninput=filter;filter();const dialog=document.querySelector('dialog'),viewer=document.querySelector('.viewer'),full=viewer.querySelector('img');for(const link of document.querySelectorAll('a.shot'))link.onclick=event=>{event.preventDefault();full.src=link.href;full.alt=link.dataset.title;document.querySelector('#image-title').textContent=link.dataset.title;document.querySelector('#original').href=link.href;viewer.classList.remove('zoom');document.querySelector('#zoom').textContent='Actual size';dialog.showModal();viewer.scrollTop=0;};document.querySelector('#close').onclick=()=>dialog.close();document.querySelector('#zoom').onclick=event=>{viewer.classList.toggle('zoom');event.target.textContent=viewer.classList.contains('zoom')?'Fit width':'Actual size';};
</script></body></html>`;
await writeFile(join(output, "index.html"), html);
await writeFile(
	join(output, "manifest.json"),
	JSON.stringify({ fixtureData: true, capturedOn, captures }, null, 2),
);

const selections = [
	["tier-cards-monthly-dark.png", "FREE · PREMIUM · PRO · MAX"],
	["upgrade-ai-budget.png", "UPGRADE DIALOG + FREE ALTERNATIVES"],
	["usage-overview-dark.png", "CURRENT USAGE + APP / MODEL BREAKDOWN"],
	["quota-warning-75-detail.png", "EARLY QUOTA WARNING"],
	files.includes("runtime-calculator-personal-detail.png")
		? ["runtime-calculator-personal-detail.png", "CLOUD RUN CALCULATOR"]
		: ["usage-cost-details.png", "AI COSTS + TOKEN DETAILS"],
	["website-pricing-desktop.png", "WEBSITE PRICING"],
];
const tileW = 840,
	tileH = 500,
	gap = 24,
	top = 120;
const composites = [];
for (let index = 0; index < selections.length; index++) {
	const [file, label] = selections[index];
	const x = gap + (index % 2) * (tileW + gap),
		y = top + Math.floor(index / 2) * (tileH + gap);
	const buffer = await sharp(join(output, file))
		.resize(tileW, tileH - 44, { fit: "contain", background: "#14171c" })
		.png()
		.toBuffer();
	composites.push({ input: buffer, left: x, top: y + 44 });
	composites.push({
		input: Buffer.from(
			`<svg width="${tileW}" height="44"><rect width="100%" height="100%" fill="#1d2027"/><text x="18" y="28" fill="#f4f4f5" font-size="16" font-family="Arial">${escape(label)}</text></svg>`,
		),
		left: x,
		top: y,
	});
}
const width = tileW * 2 + gap * 3;
composites.push({
	input: Buffer.from(
		`<svg width="${width}" height="110"><text x="24" y="46" fill="#fff" font-size="32" font-family="Arial" font-weight="bold">Flow-Like pricing update</text><text x="24" y="80" fill="#a6abb6" font-size="18" font-family="Arial">${captures.length} browser screenshots · Actual components with sample usage data</text></svg>`,
	),
	left: 0,
	top: 0,
});
await sharp({
	create: {
		width,
		height: top + tileH * 3 + gap * 3,
		channels: 4,
		background: "#0c0e12",
	},
})
	.composite(composites)
	.png()
	.toFile(join(output, "preview.png"));
console.log(`Created gallery with ${captures.length} screenshots`);
