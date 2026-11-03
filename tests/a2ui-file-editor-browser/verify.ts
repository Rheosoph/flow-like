import assert from "node:assert/strict";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { compile } from "@tailwindcss/node";
import puppeteer, { type Frame } from "puppeteer";

const root = resolve(import.meta.dir, "../..");
const ui = join(root, "packages/ui");
const directory = await mkdtemp(
	join(tmpdir(), "flow-like-file-editor-layout-"),
);
const entry = `
import React from 'react';
import {createRoot} from 'react-dom/client';
import {A2UICard} from ${JSON.stringify(join(ui, "components/a2ui/container/Card.tsx"))};
import {A2UIColumn} from ${JSON.stringify(join(ui, "components/a2ui/layout/Column.tsx"))};
import {A2UIFilePreview} from ${JSON.stringify(join(ui, "components/a2ui/display/FilePreview.tsx"))};
import {A2UIFileEditor} from ${JSON.stringify(join(ui, "components/a2ui/display/FileEditor.tsx"))};
import {BuilderRenderer} from ${JSON.stringify(join(ui, "components/builder/BuilderRenderer.tsx"))};
import {ResponsivePreview} from ${JSON.stringify(join(ui, "components/builder/ResponsivePreview.tsx"))};
import {A2UIRenderer} from ${JSON.stringify(join(ui, "components/a2ui/A2UIRenderer.tsx"))};
import {executeCraftCommand} from ${JSON.stringify(join(ui, "components/storage/craft-mcp-session.ts"))};
import {TooltipProvider} from ${JSON.stringify(join(ui, "components/ui/tooltip.tsx"))};
const app=createRoot(document.getElementById('root'));
const str=literalString=>({literalString});
const base={componentId:'preview',surfaceId:'page',renderChild:()=>null};
window.testEditorRpc=async()=>{
 const context={appId:'layout-test',deadline:Date.now()+2000};
 const session=await executeCraftCommand('craft.session',{elementRef:'page/preview'},context);
 const response=await executeCraftCommand('craft.mcp',{elementRef:'page/preview',...session,message:{jsonrpc:'2.0',id:1,method:'tools/list'}},context);
 return {session,response};
};
window.renderFixture=(mode='preview')=>{
 window.uploads=[];window.actionEvents=[];
 window.builderMode=mode.startsWith('builder');
 if(window.builderMode||mode.startsWith('responsive')){
  const editor=mode==='responsive-word'?'wordcraft':'solidcraft';
  const components=[
   {id:'card',style:mode==='builder'?{height:'calc(100vh - 40px)'}:mode==='builder-long'?{}:{className:'h-full'},component:{type:'card',title:str('CAD review'),children:{explicitList:['column']}}},
   {id:'column',style:mode==='builder-long'?{}:{className:'h-full'},component:{type:'column',gap:str('12px'),children:{explicitList:['preview','input','button']}}},
   {id:'preview',style:mode==='builder-long'?{height:'1600px',flexShrink:0}:{className:'h-full min-h-[320px]'},component:mode.startsWith('responsive')?{type:'fileEditor',editor:str(editor),src:str(editor==='wordcraft'?'/fixture.docx':'/fixture.solidcraft'),eventHandlers:{save:[{name:'workflow_event',context:{nodeId:'save'}}]}}:{type:'filePreview',src:str('/fixture.solidcraft')}},
   {id:'input',component:{type:'textField'}},{id:'button',component:{type:'button'}}
  ];
  window.builderComponents=new Map(components.map(component=>[component.id,component]));
  const surface={id:'page',rootComponentId:'card',components:Object.fromEntries(window.builderComponents),dataModel:[]};
  app.render(<TooltipProvider>{mode.startsWith('responsive')?<ResponsivePreview><div className='h-full w-full overflow-auto'><A2UIRenderer surface={surface} className='h-full min-h-full w-full'/></div></ResponsivePreview>:<div id='builder-viewport' className='h-full overflow-auto'><div data-canvas-id='layout' className='h-full min-h-full relative'><BuilderRenderer surface={surface} className='h-full w-full min-h-full'/></div></div>}</TooltipProvider>);
  return;
 }
 const fileStyle=mode==='fixed'?{height:'240px',flexShrink:0}:mode==='component-height'?{flexShrink:0}:{className:'h-full min-h-[320px]'};
 const renderChild=id=>{
  if(id==='column') return <A2UIColumn {...base} componentId='column' elementRef={el=>el&&(el.id='column')} style={{className:'h-full'}} component={{type:'column',gap:str('12px'),children:{explicitList:['preview','input','button']}}} renderChild={renderChild}/>;
  if(id==='input') return <textarea id='input' aria-label='Instructions' className='h-[70px] shrink-0 rounded-md border p-3' defaultValue='Describe the changes to the model'/>;
  if(id==='button') return <button id='button' className='h-[70px] shrink-0 rounded-md bg-primary text-primary-foreground'>Apply changes</button>;
  if(mode==='setup'||mode==='editor') return <A2UIFileEditor {...base} elementRef={el=>el&&(el.id='preview')} style={mode==='setup'?undefined:fileStyle} component={{id:'preview',type:'fileEditor',editor:str('solidcraft'),...(mode==='editor'?{eventHandlers:{save:[{name:'workflow_event',context:{nodeId:'save'}}]}}:{})}}/>;
  return <A2UIFilePreview {...base} elementRef={el=>el&&(el.id='preview')} style={fileStyle} component={{id:'preview',type:'filePreview',src:str(mode==='loading'?'/loading.solidcraft':'/fixture.solidcraft'),...(['fixed','component-height'].includes(mode)?{height:str('280px')}:{})}}/>;
 };
 app.render(<TooltipProvider><A2UICard {...base} componentId='card' elementRef={el=>el&&(el.id='card')} style={{className:'h-full'}} component={{type:'card',title:str('CAD review'),children:{explicitList:['column']}}} renderChild={renderChild}/></TooltipProvider>);
};
window.renderFixture();
`;
const shims: Record<string, string> = {
	locales:
		"export const useTranslation=()=>({t:(key,fallback)=>fallback??key}); export const i18n={t:(key,fallback)=>fallback??key};",
	data: "export const DataProvider=({children})=>children; export const DataScopeProvider=DataProvider; export const useData=()=>({resolve:value=>value&&('literalJson' in value?JSON.parse(value.literalJson):Object.values(value)[0]),setByPath:()=>{}});",
	actions:
		"export const ActionProvider=({children})=>children; export const useActionContext=()=>({appId:'layout-test',isPreviewMode:!window.builderMode}); export const useComponentEventTrigger=()=>async(name)=>{window.actionEvents.push(name)}; export const useOnAction=()=>()=>{}; export const useSetElementValue=()=>()=>{};",
	assets: "export const useAssetUrl=source=>({url:source,isLoading:false});",
	backend:
		"export const useBackend=()=>({helperState:{fileToUrl:async(file)=>{window.uploads.push({name:file.name,bytes:await file.text()});return location.origin+'/saved/'+file.name}},storageState:{}});",
	native: "export const PdfFrame=()=>null;",
	audio: "export const AudioPreview=()=>null;",
	voice: "export const AudioPlayback=()=>null;",
	builder:
		"const noop=()=>{}; export const useBuilder=()=>({selection:{componentIds:['preview']},selectComponent:noop,deleteComponents:noop,copy:noop,cut:noop,paste:noop,isComponentHidden:()=>false,components:window.builderComponents,actionContext:{appId:'layout-test'},widgetRefs:{}});",
	dnd: "const noop=()=>{}; export const useDraggable=()=>({attributes:{},listeners:{},setNodeRef:noop,setActivatorNodeRef:noop,isDragging:false}); export const useDroppable=()=>({setNodeRef:noop});",
	builderDnd:
		"export const COMPONENT_MOVE_TYPE='move'; export const useBuilderDnd=()=>({activeId:null,overData:null});",
	widgetBuilder: "export const ROOT_ID='card';",
	runtimeTailwind: "export const useRuntimeTailwindStyles=()=>{};",
	reload: "export const useMicroWidgetReload=()=>null;",
	registry: `import React from 'react';
import {A2UICard} from ${JSON.stringify(join(ui, "components/a2ui/container/Card.tsx"))};
import {A2UIColumn} from ${JSON.stringify(join(ui, "components/a2ui/layout/Column.tsx"))};
import {A2UIFilePreview} from ${JSON.stringify(join(ui, "components/a2ui/display/FilePreview.tsx"))};
import {A2UIFileEditor} from ${JSON.stringify(join(ui, "components/a2ui/display/FileEditor.tsx"))};
const renderers={card:A2UICard,column:A2UIColumn,filePreview:A2UIFilePreview,fileEditor:A2UIFileEditor,textField:({elementRef})=><textarea ref={elementRef} className='h-[70px] shrink-0 rounded-md border p-3' aria-label='Instructions' defaultValue='Describe the changes to the model'/>,button:({elementRef})=><button ref={elementRef} className='h-[70px] shrink-0 rounded-md bg-primary text-primary-foreground'>Apply changes</button>};
export const getComponentRenderer=type=>renderers[type];`,
};
const bundle = await Bun.build({
	entrypoints: ["file-editor-layout-entry"],
	target: "browser",
	format: "esm",
	define: { "process.env.NODE_ENV": '"production"' },
	plugins: [
		{
			name: "layout-fixture",
			setup(build) {
				build.onResolve({ filter: /^\.\.\/\.\.\/lib$/ }, ({ importer }) =>
					importer.endsWith("/ResponsivePreview.tsx")
						? { path: join(ui, "lib/utils.ts") }
						: undefined,
				);
				build.onResolve({ filter: /^file-editor-layout-entry$/ }, () => ({
					path: "entry",
					namespace: "fixture",
				}));
				for (const [filter, name] of [
					[/^@flow-like\/locales$/, "locales"],
					[/\/DataContext$/, "data"],
					[/\/ActionHandler$/, "actions"],
					[/\/use-asset-url$/, "assets"],
					[/\/state\/backend-state$/, "backend"],
					[/\/ui\/file-previewer$/, "native"],
					[/\/ui\/audio-preview$/, "audio"],
					[/\/voice$/, "voice"],
					[/\/BuilderContext$/, "builder"],
					[/^@dnd-kit\/core$/, "dnd"],
					[/\/BuilderDndContext$/, "builderDnd"],
					[/\/WidgetBuilder$/, "widgetBuilder"],
					[/\/use-runtime-tailwind$/, "runtimeTailwind"],
					[/\/use-micro-widget-reload$/, "reload"],
					[/\/ComponentRegistry$/, "registry"],
				] as const)
					build.onResolve({ filter }, () => ({
						path: name,
						namespace: "fixture",
					}));
				build.onLoad({ filter: /.*/, namespace: "fixture" }, ({ path }) => ({
					contents: path === "entry" ? entry : shims[path],
					loader: "tsx",
					resolveDir: root,
				}));
			},
		},
	],
});
assert.ok(bundle.success, bundle.logs.join("\n"));
const script = await bundle.outputs[0].text();
const compiler = await compile(await Bun.file(join(ui, "global.css")).text(), {
	base: ui,
	onDependency: () => {},
});
// The iframe protocol is simulated here. Separate Craft browser suites exercise the WASM editors.
const runtime = `<!doctype html><style>body{margin:0;height:100vh;background:#17202c;color:#cbd5e1;display:grid;place-items:center;font:14px system-ui}div{width:160px;height:160px;border:2px solid #60a5fa;transform:rotate(-25deg) skew(10deg)}</style><div></div><script type='module'>
import {isCraftHostSender} from '/packages/craft-viewers/mcp.mjs';
const session=new URLSearchParams(location.search).get('session');
const channel=location.pathname.startsWith('/wordcraft/')?'flow-like-wordcraft':'flow-like-craft';
const {validateCommand}=await import(channel==='flow-like-wordcraft'?'/packages/wordcraft/protocol.mjs':'/packages/craft-viewers/protocol.mjs');
const post=(type,details={},transfer=[])=>parent.postMessage({channel,session,type,...details},location.origin,transfer);
addEventListener('message',event=>{
 window.lastMessage={type:event.data?.type,parentSource:event.source===parent,outerSource:event.source===top,origin:event.origin,expectedOrigin:location.origin,channel:event.data?.channel,sessionMatches:event.data?.session===session};
 if(!isCraftHostSender(event.source,window)||event.origin!==location.origin)return;
 const message=validateCommand(event.data,session,{extensions:['solidcraft'],maxBytes:1024});
 if(!message)return;
 if(message.type==='open'&&message.name==='loading.solidcraft'){
  document.body.innerHTML='<button id="cancel">Cancel loading</button>';
  document.getElementById('cancel').onclick=()=>post('error',{error:'Opening the document was cancelled.'});
  post('loading',{interactive:true,completed:2,total:10});
 }else if(message.type==='open'||message.type==='create'){post('opened');if(!message.readOnly)post('dirty',{dirty:true})}
 if(message.type==='export'){const bytes=new TextEncoder().encode('saved draft').buffer;post('exported',{requestId:message.requestId,bytes,revision:1},[bytes])}
 if(message.type==='saved'){window.savedRevision=message.revision;post('dirty',{dirty:false})}
 if(message.type==='mcp-request')post('mcp-response',{requestId:message.requestId,response:{jsonrpc:'2.0',id:message.message.id,result:{tools:[]}}});
});post('ready');</script>`;
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	fetch(request) {
		const url = new URL(request.url);
		if (
			[
				"/packages/craft-viewers/mcp.mjs",
				"/packages/craft-viewers/protocol.mjs",
				"/packages/wordcraft/mcp.mjs",
				"/packages/wordcraft/protocol.mjs",
			].includes(url.pathname)
		)
			return new Response(Bun.file(join(root, url.pathname.slice(1))), {
				headers: { "content-type": "text/javascript" },
			});
		if (url.pathname === "/app.js")
			return new Response(script, {
				headers: { "content-type": "text/javascript" },
			});
		if (
			url.pathname.startsWith("/craft/") ||
			url.pathname.startsWith("/wordcraft/")
		)
			return new Response(runtime, {
				headers: { "content-type": "text/html" },
			});
		if (
			url.pathname === "/fixture.solidcraft" ||
			url.pathname === "/loading.solidcraft" ||
			url.pathname === "/fixture.docx"
		)
			return new Response("{}", {
				headers: { "content-type": "application/octet-stream" },
			});
		return new Response(
			`<!doctype html><html class='dark'><body style='margin:0;padding:20px;box-sizing:border-box'><main id='root' style='height:calc(100vh - 40px)'></main><script type='module' src='/app.js'></script></body></html>`,
			{ headers: { "content-type": "text/html" } },
		);
	},
});
const browser = await puppeteer.launch({ headless: true });
try {
	const page = await browser.newPage();
	const errors: string[] = [];
	page.on("pageerror", (error) => {
		errors.push(String(error));
		console.error(error);
	});
	await page.setViewport({ width: 1280, height: 1200 });
	await page.goto(`http://127.0.0.1:${server.port}`);
	await page.waitForSelector("#preview iframe");
	const candidates = new Set<string>();
	const style = await page.addStyleTag({ content: ":root{}" });
	async function refreshStyles() {
		const classes = await page.evaluate(() => {
			function collect(doc: Document): string[] {
				return [
					...Array.from(doc.querySelectorAll("[class]")).flatMap((element) => [
						...element.classList,
					]),
					...Array.from(doc.querySelectorAll("iframe")).flatMap((frame) =>
						frame.contentDocument ? collect(frame.contentDocument) : [],
					),
				];
			}
			return collect(document);
		});
		for (const name of classes) candidates.add(name);
		await style.evaluate(
			(element, css) => {
				element.textContent = css;
			},
			`${compiler.build([...candidates])}\n*{transition:none!important;animation:none!important}`,
		);
		await page.evaluate(
			() =>
				new Promise((resolve) =>
					requestAnimationFrame(() => requestAnimationFrame(resolve)),
				),
		);
	}
	async function measure() {
		return page.evaluate(() =>
			Object.fromEntries(
				["card", "column", "preview", "input", "button"].map((id) => {
					const element =
						document.getElementById(id) ??
						document.querySelector(`[data-builder-component="${id}"]`);
					if (!element) throw new Error(`Missing ${id}`);
					const rect = element.getBoundingClientRect();
					return [
						id,
						{ top: rect.top, bottom: rect.bottom, height: rect.height },
					];
				}),
			),
		);
	}
	async function verifyResponsiveEditor(frame: Frame, editor: string) {
		await frame
			.waitForFunction(
				() => {
					const save = document.querySelector<HTMLButtonElement>(
						'[data-a2ui-element-ref="page/preview"] button[aria-label^="Save"]',
					);
					return save && !save.disabled;
				},
				{ timeout: 5_000 },
			)
			.catch(async (error) => {
				const runtimeFrame = await (
					await frame.$('[data-a2ui-element-ref="page/preview"] iframe')
				)?.contentFrame();
				console.error(
					"Runtime message:",
					await runtimeFrame?.evaluate(
						() => (window as unknown as { lastMessage: unknown }).lastMessage,
					),
				);
				throw error;
			});
		const rpc = await page.evaluate(() =>
			(
				window as unknown as {
					testEditorRpc(): Promise<{
						session: { editor: string };
						response: unknown;
					}>;
				}
			).testEditorRpc(),
		);
		assert.equal(rpc.session.editor, editor);
		assert.deepEqual(rpc.response, {
			jsonrpc: "2.0",
			id: 1,
			result: { tools: [] },
		});
		const runtimeFrame = await (
			await frame.$('[data-a2ui-element-ref="page/preview"] iframe')
		)?.contentFrame();
		assert.ok(runtimeFrame);
		await refreshStyles();
		const button = await frame.evaluate(() => {
			const save = document.querySelector(
				'[data-a2ui-element-ref="page/preview"] button[aria-label^="Save"]',
			);
			if (!save) throw new Error("Save button missing");
			const rect = save.getBoundingClientRect();
			return {
				x: rect.x + rect.width / 2,
				y: rect.y + rect.height / 2,
				width: innerWidth,
				height: innerHeight,
			};
		});
		const frameBounds = await (await frame.frameElement())?.boundingBox();
		assert.ok(frameBounds);
		// Account for the responsive frame's CSS scale when clicking its toolbar.
		await page.mouse.click(
			frameBounds.x + (button.x * frameBounds.width) / button.width,
			frameBounds.y + (button.y * frameBounds.height) / button.height,
		);
		await runtimeFrame.waitForFunction(
			() =>
				(window as unknown as { savedRevision?: number }).savedRevision === 1,
			{ timeout: 5_000 },
		);
		const saved = await page.evaluate(() => ({
			uploads: (window as unknown as { uploads: unknown[] }).uploads,
			events: (window as unknown as { actionEvents: string[] }).actionEvents,
		}));
		assert.deepEqual(saved.uploads, [
			{
				name: editor === "wordcraft" ? "fixture.docx" : "fixture.solidcraft",
				bytes: "saved draft",
			},
		]);
		assert.ok(saved.events.includes("save"));
	}
	await refreshStyles();
	const full = await measure();
	assert.ok(full.preview.height > 800, JSON.stringify(full));
	assert.ok(full.button.bottom <= full.card.bottom - 20, JSON.stringify(full));
	assert.equal(full.input.height, 70);
	assert.equal(full.button.height, 70);
	await page.screenshot({ path: join(directory, "full-height.png") });
	const previous = await page.addStyleTag({
		content: "[data-slot=card-content]{flex:0 1 auto;min-height:auto}",
	});
	const broken = await measure();
	assert.ok(
		full.preview.height > broken.preview.height + 300,
		JSON.stringify({ full, broken }),
	);
	await page.screenshot({ path: join(directory, "previous-height.png") });
	await previous.evaluate((element) => element.remove());
	await page.setViewport({ width: 1280, height: 900 });
	await refreshStyles();
	const smaller = await measure();
	assert.ok(Math.abs(full.preview.height - smaller.preview.height - 300) < 2);
	assert.equal(smaller.button.height, 70);
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"fixed",
		),
	);
	await refreshStyles();
	assert.equal((await measure()).preview.height, 240);
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"component-height",
		),
	);
	await refreshStyles();
	assert.equal((await measure()).preview.height, 280);
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"setup",
		),
	);
	await page.waitForFunction(() =>
		document
			.getElementById("preview")
			?.textContent?.includes("Connect a Save workflow"),
	);
	await refreshStyles();
	assert.equal(await page.$("#preview iframe"), null);
	assert.ok((await measure()).preview.height < 300);
	await page.screenshot({ path: join(directory, "editor-setup.png") });
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"editor",
		),
	);
	await page.waitForSelector("#preview iframe");
	await refreshStyles();
	assert.ok((await measure()).preview.height > 500);
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"builder",
		),
	);
	await page.waitForSelector('[data-builder-component="preview"] iframe');
	await page.waitForSelector("[data-builder-toolbar]");
	await refreshStyles();
	const builder = await measure();
	assert.equal(builder.preview.height, smaller.preview.height);
	assert.equal(builder.input.height, 70);
	assert.equal(builder.button.height, 70);
	assert.equal(
		await page.evaluate(() => {
			const preview = document.querySelector(
				'[data-builder-component="preview"]',
			);
			const chrome = document.querySelector("[data-builder-chrome]");
			return (
				preview?.parentElement?.getAttribute("data-builder-component") ===
					"column" && chrome?.parentElement === document.body
			);
		}),
		true,
	);
	await page.screenshot({ path: join(directory, "builder-selected.png") });
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"builder-percentage",
		),
	);
	await refreshStyles();
	const builderPercentage = await measure();
	assert.equal(builderPercentage.preview.height, builder.preview.height);
	assert.equal(builderPercentage.card.height, 860);
	await page.screenshot({ path: join(directory, "builder-full-height.png") });
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"builder-long",
		),
	);
	await refreshStyles();
	const scrolling = await page.evaluate(() => {
		const viewport = document.getElementById("builder-viewport");
		const button = document.querySelector('[data-builder-component="button"]');
		if (!viewport || !button) throw new Error("Missing scrolling fixture");
		viewport.scrollTop = viewport.scrollHeight;
		return {
			scrollTop: viewport.scrollTop,
			scrollHeight: viewport.scrollHeight,
			clientHeight: viewport.clientHeight,
			buttonBottom: button.getBoundingClientRect().bottom,
			viewportBottom: viewport.getBoundingClientRect().bottom,
		};
	});
	assert.ok(scrolling.scrollTop > 800, JSON.stringify(scrolling));
	assert.ok(
		scrolling.buttonBottom <= scrolling.viewportBottom,
		JSON.stringify(scrolling),
	);
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"responsive",
		),
	);
	await refreshStyles();
	await page.waitForSelector('iframe[title="Responsive preview"]');
	const responsiveFrame = await (
		await page.$('iframe[title="Responsive preview"]')
	)?.contentFrame();
	assert.ok(responsiveFrame);
	await responsiveFrame.waitForSelector(
		'[data-a2ui-element-ref="page/preview"] iframe',
	);
	await verifyResponsiveEditor(responsiveFrame, "solidcraft");
	await refreshStyles();
	const responsive = await responsiveFrame.evaluate(() => {
		const height = (id: string) =>
			document
				.querySelector(`[data-a2ui-element-ref="page/${id}"]`)
				?.getBoundingClientRect().height;
		return {
			viewport: innerHeight,
			card: height("card"),
			preview: height("preview"),
			input: height("input"),
			button: height("button"),
		};
	});
	assert.equal(responsive.card, responsive.viewport);
	assert.ok(
		responsive.preview && responsive.preview > 500,
		JSON.stringify(responsive),
	);
	assert.equal(responsive.input, 70);
	assert.equal(responsive.button, 70);
	await page.screenshot({
		path: join(directory, "responsive-full-height.png"),
	});
	await page.click('button[title="Mobile"]');
	await responsiveFrame.waitForFunction(() => innerWidth === 375);
	await refreshStyles();
	const mobile = await responsiveFrame.evaluate(() => {
		const height = (id: string) =>
			document
				.querySelector(`[data-a2ui-element-ref="page/${id}"]`)
				?.getBoundingClientRect().height;
		return {
			viewport: innerHeight,
			card: height("card"),
			preview: height("preview"),
			input: height("input"),
			button: height("button"),
		};
	});
	assert.equal(mobile.card, mobile.viewport);
	assert.ok(mobile.preview && mobile.preview > 400, JSON.stringify(mobile));
	assert.equal(mobile.input, 70);
	assert.equal(mobile.button, 70);
	await page.screenshot({ path: join(directory, "mobile-full-height.png") });
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"responsive-word",
		),
	);
	await responsiveFrame.waitForSelector(
		'[data-a2ui-element-ref="page/preview"] iframe[src*="/wordcraft/"]',
	);
	await verifyResponsiveEditor(responsiveFrame, "wordcraft");
	await refreshStyles();
	await page.screenshot({ path: join(directory, "word-responsive.png") });
	await page.evaluate(() =>
		(window as unknown as { renderFixture(mode: string): void }).renderFixture(
			"loading",
		),
	);
	const loadingElement = await page.waitForSelector("#preview iframe");
	const loadingFrame = await loadingElement?.contentFrame();
	assert.ok(loadingFrame);
	await loadingFrame.waitForSelector("#cancel");
	await refreshStyles();
	await page.waitForSelector("#preview output.sr-only");
	// A real pointer click must reach the iframe through the host's loading state.
	await loadingFrame.click("#cancel");
	await page.waitForFunction(() =>
		document.body.textContent?.includes("Opening the document was cancelled."),
	);
	assert.equal(await page.$("#preview iframe"), null);
	assert.deepEqual(errors, []);
	console.log(
		JSON.stringify({
			passed: true,
			fullHeight: full.preview.height,
			previousHeight: broken.preview.height,
			resizedHeight: smaller.preview.height,
			builderSelectedHeight: builder.preview.height,
			builderPercentageCardHeight: builderPercentage.card.height,
			builderPercentagePreviewHeight: builderPercentage.preview.height,
			responsivePreviewHeight: responsive.preview,
			mobilePreviewHeight: mobile.preview,
			screenshots: directory,
		}),
	);
} finally {
	await browser.close();
	server.stop(true);
}
