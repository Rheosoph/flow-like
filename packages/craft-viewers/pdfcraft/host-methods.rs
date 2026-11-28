impl PdfCraftApp {
    pub fn host_create(&mut self, name: &str) -> Result<(), String> {
        let bytes = self
            .session
            .create_blank(612.0, 792.0, 1)
            .map_err(|e| e.to_string());
        self.open_new_document(name, bytes?)
    }

    pub fn host_revision(&self) -> u64 {
        self.active_ids()
            .and_then(|(_, id)| self.session.get(id))
            .map(|doc| doc.edit_generation())
            .unwrap_or(0)
    }

    pub fn host_export(
        &mut self,
        ctx: &egui::Context,
    ) -> Result<(u64, std::sync::Arc<Vec<u8>>), String> {
        let (_, id) = self.active_ids().ok_or("Open a PDF first.")?;
        if !self.commit_typing_in(ctx, id) || !self.flatten_fill_sign_for_save(ctx, id) {
            return Err("Finish the current edit before saving.".into());
        }
        let bytes = self.session.save_bytes(id).map_err(|e| e.to_string())?;
        Ok((self.host_revision(), bytes))
    }

    pub fn host_saved(
        &mut self,
        ctx: &egui::Context,
        revision: u64,
        bytes: std::sync::Arc<Vec<u8>>,
    ) -> Result<bool, String> {
        let (index, id) = self.active_ids().ok_or("Open a PDF first.")?;
        let draft = self.has_unsaved_typing(index);
        let doc = self.session.get(id).ok_or("The PDF is unavailable.")?;
        if draft || doc.edit_generation() != revision {
            return Ok(false);
        }
        self.session
            .mark_saved(id, bytes, None)
            .map_err(|e| e.to_string())?;
        if let Some(doc) = self.session.get(id) {
            self.views[index].document_changed(ctx, &doc.info);
        }
        Ok(true)
    }
}

impl PdfCraftApp {
    pub fn host_mcp(
        &mut self,
        ctx: &egui::Context,
        message: &serde_json::Value,
        read_only: bool,
    ) -> serde_json::Value {
        let mutating = message["method"] == "tools/call"
            && message["params"]["name"].as_str().is_some_and(|name| {
                pdfcraft_automation::tools()
                    .iter()
                    .any(|tool| tool.name == name && !tool.read_only)
            });
        if mutating && !read_only {
            if let Some((_, id)) = self.active_ids() {
                if !self.commit_typing_in(ctx, id) {
                    return serde_json::json!({"jsonrpc":"2.0","id":message["id"],"result":{"isError":true,"content":[{"type":"text","text":"Finish the current text edit before using editor tools."}]}});
                }
            }
        }
        let before = self.host_revision();
        let reply =
            pdfcraft_automation::mcp::McpServer::handle_live(&mut self.session, message, read_only);
        if self.host_revision() != before {
            for view in &mut self.views {
                if let Some(doc) = self.session.get(view.id) {
                    view.document_changed(ctx, &doc.info);
                }
            }
        }
        reply
    }
}
