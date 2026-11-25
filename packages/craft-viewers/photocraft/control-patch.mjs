import { rustFunction } from "../rust-source.mjs";

// The upstream channel also serves desktop windows. Keep the browser's existing
// synchronous methods on the same pinned dispatcher and parameter validation.
export function prepareBrowserControl({ original, write, replaceOnce }) {
	const base = "crates/ui-egui/src";
	let control = original(`${base}/control.rs`);
	control = replaceOnce(
		control,
		"pub use craft::automation::{ControlRequest, ControlResponse, REPLY_DEADLINE, Reply};",
		"pub use craft::automation::control::{ControlRequest, ControlResponse, REPLY_DEADLINE, Reply};\npub use Reply as Outcome;",
		"browser control types",
	);
	control = replaceOnce(
		control,
		"pub(crate) mod dialogs;\n",
		"",
		"native dialog control",
	);
	control = replaceOnce(
		control,
		"mod dispatch;",
		'#[path = "control/browser_dispatch.rs"]\nmod dispatch;',
		"browser dispatcher",
	);
	control = replaceOnce(
		control,
		"mod inspect;",
		'#[path = "control/browser_inspect.rs"]\nmod inspect;',
		"browser inspection",
	);
	control = replaceOnce(
		control,
		"mod params;",
		'#[path = "control/params.rs"]\nmod params;',
		"browser parameters",
	);
	write(`${base}/browser_control.rs`, control);

	let dispatch = original(`${base}/control/dispatch.rs`);
	const replaceFunction = (text, name, body) =>
		replaceOnce(text, rustFunction(text, name), body, `browser ${name}`);
	dispatch = replaceOnce(
		dispatch,
		"const HANDLERS: [Handler; 5] = [engine_methods, shell_methods, dialog_methods, input_methods, app_methods];",
		"const HANDLERS: [Handler; 3] = [engine_methods, shell_methods, input_methods];",
		"browser methods",
	);
	dispatch = replaceFunction(
		dispatch,
		"shell_methods",
		`fn shell_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    (req.method == "ui.set").then(|| ui_set(app, ctx, &req.params))
}`,
	);
	dispatch = replaceFunction(dispatch, "dialog_methods", "");
	dispatch = replaceFunction(
		dispatch,
		"input_methods",
		`fn input_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    (req.method == "ui.pointer").then(|| pointer(app, ctx, &req.params))
}`,
	);
	const nativeStart = dispatch.indexOf(
		"// ---- the application: open, save, quit ----",
	);
	if (nativeStart < 0)
		throw new Error("The PhotoCraft native control boundary changed.");
	write(`${base}/control/browser_dispatch.rs`, dispatch.slice(0, nativeStart));

	let inspect = original(`${base}/control/inspect.rs`);
	inspect = replaceFunction(
		inspect,
		"agent_link_json",
		'fn agent_link_json(_app: &PhotocraftApp) -> Value { json!({"mcpFlag": false, "launcher": false}) }',
	);
	inspect = replaceOnce(
		inspect,
		"craft::automation::dialogs::inspect(&app.framework.dialogs)",
		`Value::Array(app.framework.dialogs.open_dialogs().into_iter().map(|dialog| json!({
            "id": dialog.id.value().to_string(),
            "parent": dialog.parent.map(|id| id.value().to_string()),
            "key": dialog.key,
            "fields": dialog.fields,
        })).collect())`,
		"browser dialog inspection",
	);
	write(`${base}/control/browser_inspect.rs`, inspect);

	let lib = original(`${base}/lib.rs`);
	lib = replaceOnce(
		lib,
		"pub mod control;",
		'pub mod control;\n#[cfg(target_arch = "wasm32")]\n#[path = "browser_control.rs"]\npub mod control;',
		"browser control module",
	);
	lib += `
impl PhotocraftApp {
    pub fn host_cancel_synthetic(&mut self, ctx: &egui::Context) {
        craft::ui::runtime::windows::WindowHost::of(ctx).withdraw_injected(egui::ViewportId::ROOT);
    }
}
`;
	write(`${base}/lib.rs`, lib);
}
