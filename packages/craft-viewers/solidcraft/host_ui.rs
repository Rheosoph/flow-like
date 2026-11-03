use std::cell::Cell;

use crate::SolveApp;

thread_local! {
    static PREVIEW: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn read_only() -> bool {
    PREVIEW.get()
}

fn clear_input(ctx: &egui::Context) {
    ctx.input_mut(|input| {
        input.events.clear();
        input.raw.events.clear();
        input.pointer = Default::default();
        input.modifiers = egui::Modifiers::NONE;
        input.keys_down.clear();
        input.smooth_scroll_delta = egui::Vec2::ZERO;
    });
}

impl SolveApp {
    pub fn host_open_staged(
        &mut self,
        path: &str,
    ) -> Result<std::sync::mpsc::Receiver<serde_json::Value>, String> {
        let (reply, receiver) = std::sync::mpsc::channel();
        let mut request = crate::file_load::Request::new(path, crate::file_load::Kind::Open);
        request.reply = Some(reply);
        crate::file_load::begin_request(self, request)?;
        Ok(receiver)
    }

    pub fn host_tick_load(&mut self, ctx: &egui::Context) -> (usize, usize) {
        crate::file_load::tick(self, ctx);
        self.file_load.features().unwrap_or_default()
    }

    pub fn host_loading(&mut self, ui: &mut egui::Ui) {
        self.host_preview(ui);
        crate::file_load::show(self, ui.ctx());
    }

    pub fn host_preview(&mut self, ui: &mut egui::Ui) {
        PREVIEW.set(true);
        crate::viewport::show(self, ui);
        self.resolve_fit(ui.ctx());
        PREVIEW.set(false);
    }
}

pub(crate) fn overlays(ui: &mut egui::Ui, show: impl FnOnce(&mut egui::Ui)) {
    if !read_only() {
        show(ui);
        return;
    }
    let input = ui.ctx().input(Clone::clone);
    clear_input(ui.ctx());
    ui.add_enabled_ui(false, show);
    ui.ctx().input_mut(|current| *current = input);
}
