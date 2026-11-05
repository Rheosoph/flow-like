import { readFileSync } from "node:fs";
import { join } from "node:path";

export function prepareBrowserMedia({
	directory,
	original,
	write,
	replaceOnce,
}) {
	const manifest = "crates/media/Cargo.toml";
	let cargo = original(manifest);
	const native = [];
	for (const name of [
		"isobmff",
		"matroska",
		"ogg",
		"opus",
		"h264",
		"hevc",
		"vp9",
		"av1",
	]) {
		const line = `deckcraft-${name} = { workspace = true }\n`;
		cargo = replaceOnce(cargo, line, "", `${name} native dependency`);
		native.push(line.trimEnd());
	}
	const symphonia = cargo.match(
		/^# Audio decoding[^\n]*\n# as in FilmCraft\)\.\nsymphonia = [^\n]+\n/m,
	)?.[0];
	if (!symphonia)
		throw new Error("Review DeckCraft's changed audio dependencies.");
	cargo = replaceOnce(cargo, symphonia, "", "native audio decoder");
	native.push(symphonia.trimEnd());
	cargo += `\n[target.'cfg(not(target_arch = "wasm32"))'.dependencies]\n${native.join("\n")}\n\n[target.'cfg(target_arch = "wasm32")'.dependencies]\nwasm-bindgen = "0.2"\nwasm-bindgen-futures = "0.4"\njs-sys = "0.3"\nserde_json = { workspace = true }\n`;
	write(manifest, cargo);

	const libPath = "crates/media/src/lib.rs";
	let lib = original(libPath);
	for (const module of ["audio", "feed", "player", "probe", "video"]) {
		lib = replaceOnce(
			lib,
			`pub mod ${module};`,
			`#[cfg(not(target_arch = "wasm32"))]\npub mod ${module};\n#[cfg(target_arch = "wasm32")]\n#[path = "browser-${module}.rs"]\npub mod ${module};`,
			`browser ${module} module`,
		);
	}
	lib = replaceOnce(
		lib,
		"mod yuv;",
		'#[cfg(not(target_arch = "wasm32"))]\nmod yuv;\n#[cfg(target_arch = "wasm32")]\npub mod browser;',
		"browser media backend",
	);
	lib = replaceOnce(
		lib,
		"pub use video::{Frame, VideoDecoder, poster_frame, poster_png};",
		'pub use video::{Frame, poster_frame, poster_png};\n#[cfg(not(target_arch = "wasm32"))]\npub use video::VideoDecoder;',
		"native video decoder export",
	);
	write(libPath, lib);
	const audio = original("crates/media/src/audio.rs").match(
		/\/\/\/ Decoded audio:[\s\S]*?(?=\/\/\/ Bytes as a symphonia source)/,
	)?.[0];
	if (!audio?.includes("pub struct Pcm"))
		throw new Error("Review DeckCraft's changed PCM data type.");
	write("crates/media/src/browser-audio.rs", audio);
	for (const file of [
		"browser.rs",
		"browser-player.rs",
		"browser-feed.rs",
		"browser-probe.rs",
		"browser-video.rs",
		"browser-media.js",
	]) {
		write(
			`crates/media/src/${file}`,
			readFileSync(join(directory, file), "utf8"),
		);
	}

	const uiPath = "crates/ui-egui/src/media.rs";
	let ui = original(uiPath);
	ui = replaceOnce(
		ui,
		"VideoFeed::new(bytes.clone())",
		"VideoFeed::new_for_voice(bytes.clone(), key(id))",
		"browser video voice",
	);
	const tick =
		"    pub fn tick_clock(&mut self, now: f64, session: &mut Session) -> bool {";
	ui = replaceOnce(
		ui,
		tick,
		`${tick}\n        while let Some(result) = {\n            let value = deckcraft_media::browser::take_poster();\n            (!value.is_null() && !value.is_undefined()).then_some(value)\n        } {\n            if let Some(error) = deckcraft_media::browser::field(&result, "error").as_string() {\n                session.ui_requests.push(deckcraft_engine::UiRequest::Message { text: error });\n            } else {\n                let id = deckcraft_media::browser::number(&result, "shape") as u32;\n                let ms = deckcraft_media::browser::number(&result, "ms") as u64;\n                if let Err(error) = session.execute("media.posterFrame", &serde_json::json!({"id": id, "ms": ms})) {\n                    session.ui_requests.push(deckcraft_engine::UiRequest::Message { text: error.to_string() });\n                }\n            }\n        }`,
		"asynchronous poster results",
	);
	ui = replaceOnce(
		ui,
		'            "play" => self.play(doc_uid, doc, id, &clip, t, owner),',
		`            "poster" => {\n                if let Some(item) = doc.media(clip.media) { let _ = deckcraft_media::browser::request_poster(id.0, &item.data, t.unwrap_or(0.0)); }\n            }\n            "play" => self.play(doc_uid, doc, id, &clip, t, owner),`,
		"browser poster request",
	);
	ui = replaceOnce(
		ui,
		'format!("No sound: {err}")',
		'format!("Media: {err}")',
		"media capability message",
	);
	ui = replaceOnce(
		ui,
		"        self.owners.keys().any(|id| self.is_playing(*id))",
		"        deckcraft_media::browser::pending_posters() || self.owners.keys().any(|id| self.is_playing(*id))",
		"pending browser poster repaint",
	);
	write(uiPath, original(uiPath));
	write("crates/ui-egui/src/browser-media.rs", ui);

	const enginePath = "crates/engine/src/cmd/media.rs";
	let engine = original(enginePath);
	engine = replaceOnce(
		engine,
		'    let png = poster_png(&data, ms).ok_or_else(|| EngineError::Other("no frame could be decoded there".into()))?;',
		`    let png = match poster_png(&data, ms) {\n        Some(png) => png,\n        None => {\n            #[cfg(target_arch = "wasm32")]\n            {\n                s.ui_requests.push(UiRequest::Media { action: "poster".into(), shape: id, ms: Some(ms) });\n                return Ok(json!({"id": id, "pending": true}));\n            }\n            #[cfg(not(target_arch = "wasm32"))]\n            return Err(EngineError::Other("no frame could be decoded there".into()));\n        }\n    };`,
		"asynchronous poster command",
	);
	write(enginePath, engine);
}
