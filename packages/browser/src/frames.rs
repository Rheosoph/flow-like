// Derived from rustwright src/lib.rs @fca1438, Copyright (c) 2026 Ikonomos Inc (dba Skyvern), MIT; modified by Rheosoph GmbH. See NOTICE.
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use serde_json::Value;

use crate::event_log::{Event, EventCursor};
use crate::types::{
    FrameAttached, FrameDetached, FrameId, FrameInfo, FrameNavigated, FrameTreeNode, LoaderId,
    NavigatedWithinDocument, SessionId, TargetInfo,
};

pub(crate) const UTIL_WORLD: &str = "__flowlike_util__";

const REMOVED_FRAMES_KEPT: usize = 256;

#[derive(Clone, Debug)]
pub(crate) struct FrameNode {
    pub id: FrameId,
    pub parent: Option<FrameId>,
    pub session: Option<SessionId>,
    pub reported_by: SessionId,
    pub loader_id: Option<LoaderId>,
    pub url: String,
    pub url_fragment: Option<String>,
    pub name: Option<String>,
    pub children: Vec<FrameId>,
    pub last_seq: u64,
    document_seq: u64,
}

impl FrameNode {
    fn new(id: FrameId, parent: Option<FrameId>, reported_by: SessionId) -> Self {
        Self {
            id,
            parent,
            session: None,
            reported_by,
            loader_id: None,
            url: String::new(),
            url_fragment: None,
            name: None,
            children: Vec::new(),
            last_seq: 0,
            document_seq: 0,
        }
    }

    fn commit(&mut self, frame: &FrameInfo, session: &SessionId, seq: u64) {
        self.commit_document(frame, session, seq);
        self.commit_url(frame, seq);
    }

    fn commit_document(&mut self, frame: &FrameInfo, session: &SessionId, seq: u64) {
        self.loader_id = known_loader(&frame.loader_id);
        self.name = frame.name.clone();
        self.reported_by = session.clone();
        self.document_seq = seq;
    }

    fn commit_url(&mut self, frame: &FrameInfo, seq: u64) {
        let (url, embedded_fragment) = split_fragment(&frame.url);
        self.url = url;
        self.url_fragment = frame
            .url_fragment
            .clone()
            .filter(|fragment| !fragment.is_empty())
            .or(embedded_fragment);
        self.last_seq = seq;
    }

    fn forget_document(&mut self, seq: u64) {
        self.loader_id = None;
        self.last_seq = seq;
        self.document_seq = seq;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetachResolution {
    ChildDisposed,
    NeedsRoundTrip,
    SwappedBack,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameLookup {
    Missing,
    InTransit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Release {
    SwappedBack,
    Superseded,
}

pub(crate) struct FrameTree {
    main: FrameId,
    page_session: SessionId,
    nodes: HashMap<FrameId, FrameNode>,
    pending_detach: HashSet<SessionId>,
    released: HashMap<SessionId, Release>,
    removed: VecDeque<(FrameId, u64)>,
    latest_seq: u64,
}

fn split_fragment(url: &str) -> (String, Option<String>) {
    match url.find('#') {
        Some(index) => (url[..index].to_owned(), Some(url[index..].to_owned())),
        None => (url.to_owned(), None),
    }
}

pub(crate) fn known_loader(loader: &LoaderId) -> Option<LoaderId> {
    (!loader.as_str().is_empty()).then(|| loader.clone())
}

impl FrameTree {
    pub(crate) fn new(main: FrameId, page_session: SessionId) -> Self {
        let mut node = FrameNode::new(main.clone(), None, page_session.clone());
        node.session = Some(page_session.clone());
        Self {
            nodes: HashMap::from([(main.clone(), node)]),
            main,
            page_session,
            pending_detach: HashSet::new(),
            released: HashMap::new(),
            removed: VecDeque::new(),
            latest_seq: 0,
        }
    }

    pub(crate) fn seed(&mut self, session: &SessionId, tree: &FrameTreeNode, cursor: EventCursor) {
        let owns_root = self
            .nodes
            .get(&tree.frame.id)
            .is_some_and(|root| root.session.as_ref() == Some(session));
        if !owns_root {
            return;
        }
        let mut stack: Vec<&FrameTreeNode> = vec![tree];
        while let Some(entry) = stack.pop() {
            if self.seed_frame(session, &entry.frame, cursor) {
                stack.extend(entry.child_frames.iter().rev());
            }
        }
    }

    /// Returns whether the reply still describes the current document of the frame, so that its
    /// children merge too. Same-document events newer than the cursor keep the URL but leave the
    /// document fields to the reply.
    fn seed_frame(&mut self, session: &SessionId, frame: &FrameInfo, cursor: EventCursor) -> bool {
        let url_is_older = match self.nodes.get(&frame.id) {
            None if self.removed_since(&frame.id, cursor.0) => return false,
            None => {
                self.insert(FrameNode::new(
                    frame.id.clone(),
                    frame.parent_id.clone(),
                    session.clone(),
                ));
                true
            }
            Some(_) if self.is_stale(&frame.id, session) => return false,
            Some(node) if node.document_seq >= cursor.0 => {
                return node.loader_id == known_loader(&frame.loader_id);
            }
            Some(node) => node.last_seq < cursor.0,
        };
        if let Some(parent) = &frame.parent_id {
            self.reparent(&frame.id, parent);
        }
        let seq = cursor.0.saturating_sub(1);
        if let Some(node) = self.nodes.get_mut(&frame.id) {
            node.commit_document(frame, session, seq);
            if url_is_older {
                node.commit_url(frame, seq);
            }
        }
        true
    }

    pub(crate) fn on_event(&mut self, event: &Event) -> bool {
        let Some(session) = event.session.as_ref() else {
            return false;
        };
        self.latest_seq = self.latest_seq.max(event.seq);
        match &*event.method {
            "Page.frameAttached" => event
                .decode::<FrameAttached>()
                .is_some_and(|attached| self.frame_attached(session, &attached, event.seq)),
            "Page.frameDetached" => event
                .decode::<FrameDetached>()
                .is_some_and(|detached| self.frame_detached(session, &detached, event.seq)),
            "Page.frameNavigated" => event
                .decode::<FrameNavigated>()
                .is_some_and(|navigated| self.frame_navigated(session, &navigated, event.seq)),
            "Page.navigatedWithinDocument" => event
                .decode::<NavigatedWithinDocument>()
                .is_some_and(|navigated| {
                    self.navigated_within_document(session, &navigated, event.seq)
                }),
            _ => false,
        }
    }

    fn frame_attached(&mut self, session: &SessionId, attached: &FrameAttached, seq: u64) -> bool {
        let id = &attached.frame_id;
        if *id == self.main {
            return false;
        }
        match self.nodes.get(id).map(|node| node.session.clone()) {
            Some(Some(child)) if child != *session => {
                self.swap_back(id, child, seq);
                true
            }
            Some(_) => false,
            None if self.accepts_new_frame(session, &attached.parent_frame_id) => {
                let mut node = FrameNode::new(
                    id.clone(),
                    Some(attached.parent_frame_id.clone()),
                    session.clone(),
                );
                node.forget_document(seq);
                self.insert(node);
                true
            }
            None => false,
        }
    }

    fn swap_back(&mut self, id: &FrameId, child: SessionId, seq: u64) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.session = None;
            node.forget_document(seq);
        }
        self.drop_descendants(id);
        self.released.insert(child, Release::SwappedBack);
    }

    fn frame_detached(&mut self, session: &SessionId, detached: &FrameDetached, seq: u64) -> bool {
        let id = &detached.frame_id;
        let local = self
            .nodes
            .get(id)
            .is_some_and(|node| node.session.is_none());
        if !local || self.is_stale(id, session) {
            return false;
        }
        if detached.reason == "swap" {
            if let Some(node) = self.nodes.get_mut(id) {
                node.forget_document(seq);
            }
            self.drop_descendants(id);
        } else {
            self.remove_subtree(id, seq);
        }
        true
    }

    fn frame_navigated(
        &mut self,
        session: &SessionId,
        navigated: &FrameNavigated,
        seq: u64,
    ) -> bool {
        let frame = &navigated.frame;
        if self.nodes.contains_key(&frame.id) {
            if self.is_stale(&frame.id, session) {
                return false;
            }
        } else if !self.removed_since(&frame.id, 0)
            && frame
                .parent_id
                .as_ref()
                .is_some_and(|parent| self.accepts_new_frame(session, parent))
        {
            self.insert(FrameNode::new(
                frame.id.clone(),
                frame.parent_id.clone(),
                session.clone(),
            ));
        } else {
            return false;
        }
        if let Some(node) = self.nodes.get_mut(&frame.id) {
            node.commit(frame, session, seq);
        }
        self.drop_descendants(&frame.id);
        true
    }

    fn navigated_within_document(
        &mut self,
        session: &SessionId,
        navigated: &NavigatedWithinDocument,
        seq: u64,
    ) -> bool {
        if self.is_stale(&navigated.frame_id, session) {
            return false;
        }
        let Some(node) = self.nodes.get_mut(&navigated.frame_id) else {
            return false;
        };
        let (url, fragment) = split_fragment(&navigated.url);
        node.url = url;
        node.url_fragment = fragment;
        node.last_seq = seq;
        true
    }

    pub(crate) fn on_child_session_attached(
        &mut self,
        parent: &SessionId,
        child: &SessionId,
        info: &TargetInfo,
        seq: u64,
    ) -> bool {
        let id = FrameId::new(info.target_id.as_str());
        if id == self.main {
            return false;
        }
        self.latest_seq = self.latest_seq.max(seq);
        self.pending_detach.remove(child);
        self.released.remove(child);
        self.place_child_root(&id, parent, info);
        let previous = self.nodes.get_mut(&id).and_then(|node| {
            node.forget_document(seq);
            node.session.replace(child.clone())
        });
        if let Some(previous) = previous.filter(|previous| previous != child) {
            self.released.insert(previous, Release::Superseded);
        }
        self.drop_descendants(&id);
        true
    }

    fn place_child_root(&mut self, id: &FrameId, parent: &SessionId, info: &TargetInfo) {
        if !self.nodes.contains_key(id) {
            self.insert(FrameNode::new(
                id.clone(),
                info.parent_frame_id.clone(),
                parent.clone(),
            ));
        } else if let Some(parent_frame) = &info.parent_frame_id {
            self.reparent(id, parent_frame);
        }
    }

    pub(crate) fn on_child_session_detached(&mut self, child: &SessionId) -> DetachResolution {
        if self.released.remove(child).is_some() {
            self.pending_detach.remove(child);
            return DetachResolution::ChildDisposed;
        }
        if self.roots_of(child).is_empty() {
            return DetachResolution::Removed;
        }
        self.pending_detach.insert(child.clone());
        DetachResolution::NeedsRoundTrip
    }

    pub(crate) fn resolve_pending_detach(&mut self, child: &SessionId) -> DetachResolution {
        self.pending_detach.remove(child);
        match self.released.remove(child) {
            Some(Release::SwappedBack) => DetachResolution::SwappedBack,
            Some(Release::Superseded) => DetachResolution::ChildDisposed,
            None => {
                let seq = self.latest_seq;
                for root in self.roots_of(child) {
                    self.remove_subtree(&root, seq);
                }
                DetachResolution::Removed
            }
        }
    }

    pub(crate) fn main_id(&self) -> &FrameId {
        &self.main
    }

    pub(crate) fn get(&self, id: &FrameId) -> Option<&FrameNode> {
        self.nodes.get(id)
    }

    pub(crate) fn session_for_frame(&self, id: &FrameId) -> Result<SessionId, FrameLookup> {
        let session = self
            .local_root_node(id)
            .and_then(|root| root.session.clone())
            .ok_or(FrameLookup::Missing)?;
        if self.pending_detach.contains(&session) {
            return Err(FrameLookup::InTransit);
        }
        Ok(session)
    }

    pub(crate) fn local_root(&self, id: &FrameId) -> Result<FrameId, FrameLookup> {
        let root = self.local_root_node(id).ok_or(FrameLookup::Missing)?;
        let in_transit = root
            .session
            .as_ref()
            .is_some_and(|session| self.pending_detach.contains(session));
        if in_transit {
            return Err(FrameLookup::InTransit);
        }
        Ok(root.id.clone())
    }

    /// The local root of the session first, then its same-process descendants in document order.
    pub(crate) fn frames_of_session(&self, session: &SessionId) -> Vec<FrameId> {
        self.roots_of(session)
            .iter()
            .flat_map(|root| self.preorder(root, |node| node.session.is_none()))
            .collect()
    }

    pub(crate) fn committed_loader(&self, id: &FrameId) -> Option<LoaderId> {
        self.nodes.get(id).and_then(|node| node.loader_id.clone())
    }

    pub(crate) fn main_url(&self) -> String {
        self.nodes
            .get(&self.main)
            .map(|node| format!("{}{}", node.url, node.url_fragment.as_deref().unwrap_or("")))
            .unwrap_or_default()
    }

    pub(crate) fn descendants_preorder(&self, id: &FrameId) -> Vec<FrameId> {
        let mut ordered = self.preorder(id, |_| true);
        if !ordered.is_empty() {
            ordered.remove(0);
        }
        ordered
    }

    pub(crate) fn in_transit(&self, id: &FrameId) -> bool {
        matches!(self.session_for_frame(id), Err(FrameLookup::InTransit))
    }

    fn preorder(&self, root: &FrameId, descend: impl Fn(&FrameNode) -> bool) -> Vec<FrameId> {
        let mut ordered = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![root.clone()];
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            if !seen.insert(id.clone()) {
                continue;
            }
            ordered.push(id);
            let children = node
                .children
                .iter()
                .rev()
                .filter(|child| self.nodes.get(*child).is_some_and(&descend));
            stack.extend(children.cloned());
        }
        ordered
    }

    fn local_root_node(&self, id: &FrameId) -> Option<&FrameNode> {
        let mut node = self.nodes.get(id)?;
        for _ in 0..=self.nodes.len() {
            if node.session.is_some() {
                return Some(node);
            }
            node = self.nodes.get(node.parent.as_ref()?)?;
        }
        None
    }

    fn owner_session(&self, id: &FrameId) -> Option<&SessionId> {
        self.local_root_node(id)
            .and_then(|root| root.session.as_ref())
    }

    fn is_stale(&self, id: &FrameId, session: &SessionId) -> bool {
        self.owner_session(id).is_some_and(|owner| owner != session)
    }

    fn is_live(&self, session: &SessionId) -> bool {
        *session == self.page_session
            || self
                .nodes
                .values()
                .any(|node| node.session.as_ref() == Some(session))
    }

    fn accepts_new_frame(&self, session: &SessionId, parent: &FrameId) -> bool {
        if self.nodes.contains_key(parent) {
            !self.is_stale(parent, session)
        } else {
            self.is_live(session) && !self.removed_since(parent, 0)
        }
    }

    fn roots_of(&self, session: &SessionId) -> Vec<FrameId> {
        let mut roots: Vec<FrameId> = self
            .nodes
            .values()
            .filter(|node| node.session.as_ref() == Some(session))
            .map(|node| node.id.clone())
            .collect();
        roots.sort();
        roots
    }

    fn removed_since(&self, id: &FrameId, seq: u64) -> bool {
        self.removed
            .iter()
            .any(|(removed, at)| removed == id && *at >= seq)
    }

    fn insert(&mut self, node: FrameNode) {
        let id = node.id.clone();
        if let Some(parent) = node.parent.clone() {
            self.nodes.insert(id.clone(), node);
            self.link_child(&parent, &id);
        } else {
            self.nodes.insert(id.clone(), node);
        }
        let mut orphans: Vec<(u64, FrameId)> = self
            .nodes
            .values()
            .filter(|orphan| orphan.parent.as_ref() == Some(&id))
            .map(|orphan| (orphan.last_seq, orphan.id.clone()))
            .collect();
        orphans.sort();
        for (_, orphan) in orphans {
            self.link_child(&id, &orphan);
        }
    }

    fn link_child(&mut self, parent: &FrameId, child: &FrameId) {
        if let Some(parent) = self.nodes.get_mut(parent)
            && !parent.children.contains(child)
        {
            parent.children.push(child.clone());
        }
    }

    fn reparent(&mut self, id: &FrameId, parent: &FrameId) {
        let current = self.nodes.get(id).and_then(|node| node.parent.clone());
        if current.as_ref() == Some(parent) {
            self.link_child(parent, id);
            return;
        }
        if self.is_ancestor_or_self(id, parent) {
            return;
        }
        if let Some(old) = current.and_then(|old| self.nodes.get_mut(&old)) {
            old.children.retain(|child| child != id);
        }
        if let Some(node) = self.nodes.get_mut(id) {
            node.parent = Some(parent.clone());
        }
        self.link_child(parent, id);
    }

    fn is_ancestor_or_self(&self, ancestor: &FrameId, id: &FrameId) -> bool {
        let mut current = Some(id.clone());
        for _ in 0..=self.nodes.len() {
            match current {
                None => return false,
                Some(ref frame) if frame == ancestor => return true,
                Some(frame) => {
                    current = self.nodes.get(&frame).and_then(|node| node.parent.clone());
                }
            }
        }
        true
    }

    fn drop_descendants(&mut self, id: &FrameId) {
        for descendant in self.descendants_preorder(id) {
            self.nodes.remove(&descendant);
        }
        if let Some(node) = self.nodes.get_mut(id) {
            node.children.clear();
        }
    }

    fn remove_subtree(&mut self, id: &FrameId, seq: u64) {
        let mut removed = self.descendants_preorder(id);
        let parent = self.nodes.get(id).and_then(|node| node.parent.clone());
        if let Some(parent) = parent.and_then(|parent| self.nodes.get_mut(&parent)) {
            parent.children.retain(|child| child != id);
        }
        removed.push(id.clone());
        for frame in removed {
            self.nodes.remove(&frame);
            if self.removed.len() == REMOVED_FRAMES_KEPT {
                self.removed.pop_front();
            }
            self.removed.push_back((frame, seq));
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ContextRef {
    pub session: SessionId,
    pub id: i64,
    pub unique_id: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum World {
    Main,
    Util,
}

struct ContextEntry {
    context: ContextRef,
    frame: FrameId,
    world: World,
    order: u64,
}

pub(crate) struct ExecutionContexts {
    entries: HashMap<Arc<str>, ContextEntry>,
    atoms: HashSet<Arc<str>>,
    created: u64,
}

impl ExecutionContexts {
    pub(crate) fn new() -> Self {
        Self {
            entries: HashMap::new(),
            atoms: HashSet::new(),
            created: 0,
        }
    }

    pub(crate) fn on_event(&mut self, event: &Event) -> bool {
        let Some(session) = event.session.as_ref() else {
            return false;
        };
        match &*event.method {
            "Runtime.executionContextCreated" => self.created(session, &event.params["context"]),
            "Runtime.executionContextDestroyed" => event.params["executionContextUniqueId"]
                .as_str()
                .is_some_and(|unique_id| self.remove(unique_id)),
            "Runtime.executionContextsCleared" | "Inspector.targetCrashed" => {
                self.clear_session(session)
            }
            "Target.detachedFromTarget" => event.params["sessionId"]
                .as_str()
                .is_some_and(|child| self.clear_session(&SessionId::from(child))),
            _ => false,
        }
    }

    fn created(&mut self, session: &SessionId, context: &Value) -> bool {
        let (Some(id), Some(unique_id), Some(frame)) = (
            context["id"].as_i64(),
            context["uniqueId"].as_str(),
            context["auxData"]["frameId"].as_str(),
        ) else {
            return false;
        };
        let world = if context["auxData"]["isDefault"].as_bool() == Some(true) {
            World::Main
        } else if context["name"].as_str() == Some(UTIL_WORLD) {
            World::Util
        } else {
            return false;
        };
        self.created += 1;
        let unique_id: Arc<str> = Arc::from(unique_id);
        let entry = ContextEntry {
            context: ContextRef {
                session: session.clone(),
                id,
                unique_id: unique_id.clone(),
            },
            frame: FrameId::from(frame),
            world,
            order: self.created,
        };
        self.entries.insert(unique_id, entry);
        true
    }

    fn remove(&mut self, unique_id: &str) -> bool {
        self.atoms.remove(unique_id);
        self.entries.remove(unique_id).is_some()
    }

    fn clear_session(&mut self, session: &SessionId) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|_, entry| entry.context.session != *session);
        self.forget_dead_atoms();
        before != self.entries.len()
    }

    fn forget_dead_atoms(&mut self) {
        let entries = &self.entries;
        self.atoms
            .retain(|unique_id| entries.contains_key(unique_id));
    }

    fn find(&self, frame: &FrameId, owner: &SessionId, world: World) -> Option<ContextRef> {
        self.entries
            .values()
            .filter(|entry| {
                entry.world == world && entry.frame == *frame && entry.context.session == *owner
            })
            .max_by_key(|entry| entry.order)
            .map(|entry| entry.context.clone())
    }

    pub(crate) fn main(&self, frame: &FrameId, owner: &SessionId) -> Option<ContextRef> {
        self.find(frame, owner, World::Main)
    }

    pub(crate) fn util(&self, frame: &FrameId, owner: &SessionId) -> Option<ContextRef> {
        self.find(frame, owner, World::Util)
    }

    pub(crate) fn forget_frame(&mut self, frame: &FrameId, keep: &SessionId) {
        self.entries
            .retain(|_, entry| entry.frame != *frame || entry.context.session == *keep);
        self.forget_dead_atoms();
    }

    pub(crate) fn atoms_installed(&self, unique_id: &str) -> bool {
        self.atoms.contains(unique_id)
    }

    pub(crate) fn mark_atoms_installed(&mut self, unique_id: &str) {
        if let Some((key, _)) = self.entries.get_key_value(unique_id) {
            self.atoms.insert(key.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MAIN: &str = "T1";
    const PAGE: &str = "S1";

    fn event(seq: u64, method: &str, session: &str, params: Value) -> Event {
        Event {
            seq,
            method: method.into(),
            session: Some(SessionId::from(session)),
            params: Arc::new(params),
            received: tokio::time::Instant::now(),
        }
    }

    fn id(value: &str) -> FrameId {
        FrameId::from(value)
    }

    fn sid(value: &str) -> SessionId {
        SessionId::from(value)
    }

    fn ids(values: &[&str]) -> Vec<FrameId> {
        values.iter().map(|value| id(value)).collect()
    }

    fn page() -> FrameTree {
        let mut tree = FrameTree::new(id(MAIN), sid(PAGE));
        navigated(&mut tree, 1, PAGE, MAIN, None, "L1", "http://a/p");
        tree
    }

    fn navigated(
        tree: &mut FrameTree,
        seq: u64,
        session: &str,
        frame: &str,
        parent: Option<&str>,
        loader: &str,
        url: &str,
    ) -> bool {
        tree.on_event(&event(
            seq,
            "Page.frameNavigated",
            session,
            json!({"frame": {"id": frame, "parentId": parent, "loaderId": loader, "url": url}, "type": "Navigation"}),
        ))
    }

    fn attached(tree: &mut FrameTree, seq: u64, session: &str, frame: &str, parent: &str) -> bool {
        tree.on_event(&event(
            seq,
            "Page.frameAttached",
            session,
            json!({"frameId": frame, "parentFrameId": parent}),
        ))
    }

    fn detached(tree: &mut FrameTree, seq: u64, session: &str, frame: &str, reason: &str) -> bool {
        tree.on_event(&event(
            seq,
            "Page.frameDetached",
            session,
            json!({"frameId": frame, "reason": reason}),
        ))
    }

    fn within_document(
        tree: &mut FrameTree,
        seq: u64,
        session: &str,
        frame: &str,
        url: &str,
    ) -> bool {
        tree.on_event(&event(
            seq,
            "Page.navigatedWithinDocument",
            session,
            json!({"frameId": frame, "url": url, "navigationType": "fragment"}),
        ))
    }

    fn attach_child(
        tree: &mut FrameTree,
        seq: u64,
        parent: &str,
        child: &str,
        frame: &str,
        parent_frame: &str,
    ) {
        let info = TargetInfo {
            target_id: frame.into(),
            parent_frame_id: Some(parent_frame.into()),
            ..TargetInfo::default()
        };
        assert!(tree.on_child_session_attached(&sid(parent), &sid(child), &info, seq));
    }

    fn local_child(tree: &mut FrameTree, seq: u64, frame: &str, loader: &str) {
        assert!(attached(tree, seq, PAGE, frame, MAIN));
        assert!(navigated(
            tree,
            seq + 1,
            PAGE,
            frame,
            Some(MAIN),
            loader,
            "http://a/child"
        ));
    }

    fn tree_json(value: Value) -> FrameTreeNode {
        serde_json::from_value(value).expect("a valid Page.getFrameTree reply")
    }

    fn context(session: &str, frame: &str, unique: &str, util: bool) -> Event {
        event(
            1,
            "Runtime.executionContextCreated",
            session,
            json!({"context": {
                "id": 1, "uniqueId": unique, "name": if util { UTIL_WORLD } else { "" },
                "auxData": {"frameId": frame, "isDefault": !util}
            }}),
        )
    }

    #[test]
    fn nested_oopif_attachment_preserves_parent_session_root() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "session-outer", "outer", MAIN);
        assert!(attached(&mut tree, 3, "session-outer", "inner", "outer"));
        assert_eq!(
            tree.session_for_frame(&id("inner")),
            Ok(sid("session-outer"))
        );
        assert_eq!(tree.local_root(&id("inner")), Ok(id("outer")));
        assert_eq!(
            tree.frames_of_session(&sid("session-outer")),
            ids(&["outer", "inner"])
        );

        attach_child(
            &mut tree,
            4,
            "session-outer",
            "session-inner",
            "inner",
            "outer",
        );
        assert_eq!(
            tree.frames_of_session(&sid("session-outer")),
            ids(&["outer"])
        );
        assert_eq!(
            tree.frames_of_session(&sid("session-inner")),
            ids(&["inner"])
        );
        assert_eq!(tree.get(&id("inner")).unwrap().parent, Some(id("outer")));
        assert_eq!(tree.local_root(&id("outer")), Ok(id("outer")));
        assert_eq!(
            tree.session_for_frame(&id("outer")),
            Ok(sid("session-outer"))
        );

        assert!(attached(&mut tree, 5, "session-inner", "leaf", "inner"));
        assert_eq!(
            tree.session_for_frame(&id("leaf")),
            Ok(sid("session-inner"))
        );
        assert_eq!(
            tree.frames_of_session(&sid("session-inner")),
            ids(&["inner", "leaf"])
        );
        assert_eq!(
            tree.descendants_preorder(&id(MAIN)),
            ids(&["outer", "inner", "leaf"])
        );
    }

    #[test]
    fn a_nested_oopif_swapped_back_rejoins_its_parent_session() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "session-outer", "outer", MAIN);
        assert!(attached(&mut tree, 3, "session-outer", "inner", "outer"));
        attach_child(
            &mut tree,
            4,
            "session-outer",
            "session-inner",
            "inner",
            "outer",
        );
        assert!(attached(&mut tree, 5, "session-inner", "leaf", "inner"));

        assert_eq!(
            tree.on_child_session_detached(&sid("session-inner")),
            DetachResolution::NeedsRoundTrip
        );
        assert!(tree.in_transit(&id("inner")));
        assert!(tree.in_transit(&id("leaf")));
        assert!(!tree.in_transit(&id("outer")));
        assert!(attached(&mut tree, 6, "session-outer", "inner", "outer"));
        assert_eq!(
            tree.resolve_pending_detach(&sid("session-inner")),
            DetachResolution::SwappedBack
        );
        assert_eq!(
            tree.session_for_frame(&id("inner")),
            Ok(sid("session-outer"))
        );
        assert_eq!(tree.local_root(&id("inner")), Ok(id("outer")));
        assert_eq!(
            tree.frames_of_session(&sid("session-outer")),
            ids(&["outer", "inner"])
        );
        assert!(tree.frames_of_session(&sid("session-inner")).is_empty());
        assert!(
            tree.get(&id("leaf")).is_none(),
            "the swapped-back frame loads a new document, so the old leaf is gone"
        );
    }

    #[test]
    fn seeding_keeps_child_session_frames_and_prunes_nothing() {
        let mut tree = page();
        local_child(&mut tree, 2, "stale-child", "L2");
        attach_child(&mut tree, 4, PAGE, "session-oopif", "oopif-child", MAIN);
        let reply =
            tree_json(json!({"frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p"}}));
        tree.seed(&sid(PAGE), &reply, EventCursor(5));
        assert!(
            tree.get(&id("stale-child")).is_some(),
            "seeding is merge-only"
        );
        assert_eq!(
            tree.session_for_frame(&id("oopif-child")),
            Ok(sid("session-oopif"))
        );
    }

    #[test]
    fn an_oopif_seed_takes_over_its_descendants() {
        let mut tree = page();
        local_child(&mut tree, 2, "outer", "L2");
        assert!(attached(&mut tree, 4, PAGE, "nested", "outer"));
        attach_child(&mut tree, 5, PAGE, "session-oopif", "outer", MAIN);
        assert!(tree.get(&id("nested")).is_none());
        let reply = tree_json(json!({
            "frame": {"id": "outer", "parentId": MAIN, "loaderId": "L3", "url": "http://b/outer"},
            "childFrames": [{"frame": {"id": "nested", "parentId": "outer", "loaderId": "L4", "url": "http://b/nested"}}]
        }));
        tree.seed(&sid("session-oopif"), &reply, EventCursor(6));
        assert_eq!(
            tree.session_for_frame(&id("nested")),
            Ok(sid("session-oopif"))
        );
        assert_eq!(
            tree.frames_of_session(&sid("session-oopif")),
            ids(&["outer", "nested"])
        );
        assert_eq!(
            tree.committed_loader(&id("outer")),
            Some(LoaderId::from("L3"))
        );
        assert_eq!(
            tree.get(&id("nested")).unwrap().reported_by,
            sid("session-oopif")
        );
    }

    #[test]
    fn a_parent_seed_after_a_swap_back_reclaims_the_subtree() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "session-oopif", "outer", MAIN);
        assert!(attached(&mut tree, 3, "session-oopif", "nested", "outer"));
        assert_eq!(
            tree.on_child_session_detached(&sid("session-oopif")),
            DetachResolution::NeedsRoundTrip
        );
        assert!(attached(&mut tree, 4, PAGE, "outer", MAIN));
        let reply = tree_json(json!({
            "frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p"},
            "childFrames": [{
                "frame": {"id": "outer", "parentId": MAIN, "loaderId": "L5", "url": "http://a/outer"},
                "childFrames": [{"frame": {"id": "nested", "parentId": "outer", "loaderId": "L6", "url": "http://a/nested"}}]
            }]
        }));
        tree.seed(&sid(PAGE), &reply, EventCursor(5));
        assert_eq!(
            tree.resolve_pending_detach(&sid("session-oopif")),
            DetachResolution::SwappedBack
        );
        assert_eq!(tree.session_for_frame(&id("outer")), Ok(sid(PAGE)));
        assert_eq!(tree.session_for_frame(&id("nested")), Ok(sid(PAGE)));
        assert_eq!(
            tree.committed_loader(&id("outer")),
            Some(LoaderId::from("L5"))
        );
        assert_eq!(
            tree.frames_of_session(&sid(PAGE)),
            ids(&[MAIN, "outer", "nested"])
        );
    }

    #[test]
    fn local_to_remote_attach_first_forgets_the_parent_contexts() {
        let mut tree = page();
        let mut contexts = ExecutionContexts::new();
        local_child(&mut tree, 2, "F2", "L2");
        assert!(attached(&mut tree, 4, PAGE, "F3", "F2"));
        contexts.on_event(&context(PAGE, "F2", "parent-main", false));
        contexts.on_event(&context(PAGE, "F2", "parent-util", true));
        assert!(contexts.main(&id("F2"), &sid(PAGE)).is_some());

        attach_child(&mut tree, 5, PAGE, "S2", "F2", MAIN);
        contexts.forget_frame(&id("F2"), &sid("S2"));
        assert_eq!(tree.committed_loader(&id("F2")), None);
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid("S2")));
        assert!(tree.get(&id("F3")).is_none());
        assert!(contexts.main(&id("F2"), &sid(PAGE)).is_none());
        assert!(contexts.util(&id("F2"), &sid(PAGE)).is_none());

        assert!(
            !detached(&mut tree, 6, PAGE, "F2", "swap"),
            "the swap already happened"
        );
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid("S2")));
        assert!(navigated(
            &mut tree,
            7,
            "S2",
            "F2",
            Some(MAIN),
            "L3",
            "http://b/child"
        ));
        assert_eq!(tree.committed_loader(&id("F2")), Some(LoaderId::from("L3")));
        contexts.on_event(&context("S2", "F2", "child-util", true));
        assert_eq!(
            &*contexts.util(&id("F2"), &sid("S2")).unwrap().unique_id,
            "child-util"
        );
    }

    #[test]
    fn local_to_remote_swap_first_keeps_the_frame_for_the_new_session() {
        let mut tree = page();
        local_child(&mut tree, 2, "F2", "L2");
        assert!(attached(&mut tree, 4, PAGE, "F3", "F2"));
        assert!(detached(&mut tree, 5, PAGE, "F2", "swap"));
        assert!(tree.get(&id("F2")).is_some());
        assert!(tree.get(&id("F3")).is_none());
        assert_eq!(tree.committed_loader(&id("F2")), None);
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid(PAGE)));
        attach_child(&mut tree, 6, PAGE, "S2", "F2", MAIN);
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid("S2")));
        assert_eq!(tree.descendants_preorder(&id(MAIN)), ids(&["F2"]));
    }

    #[test]
    fn remote_to_local_attach_first_disposes_the_child() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S2", "F2", MAIN);
        assert!(navigated(
            &mut tree,
            3,
            "S2",
            "F2",
            Some(MAIN),
            "L2",
            "http://b/child"
        ));
        assert!(attached(&mut tree, 4, PAGE, "F2", MAIN));
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid(PAGE)));
        assert_eq!(tree.committed_loader(&id("F2")), None);
        assert_eq!(
            tree.on_child_session_detached(&sid("S2")),
            DetachResolution::ChildDisposed
        );
        assert!(!tree.in_transit(&id("F2")));
        assert!(navigated(
            &mut tree,
            5,
            PAGE,
            "F2",
            Some(MAIN),
            "L3",
            "http://a/back"
        ));
        assert_eq!(tree.committed_loader(&id("F2")), Some(LoaderId::from("L3")));
    }

    #[test]
    fn remote_to_local_detach_first_needs_a_round_trip() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S2", "F2", MAIN);
        assert!(navigated(
            &mut tree,
            3,
            "S2",
            "F2",
            Some(MAIN),
            "L2",
            "http://b/child"
        ));
        assert!(attached(&mut tree, 4, "S2", "F4", "F2"));
        assert_eq!(
            tree.on_child_session_detached(&sid("S2")),
            DetachResolution::NeedsRoundTrip
        );
        assert!(tree.in_transit(&id("F2")));
        assert!(tree.in_transit(&id("F4")));
        assert_eq!(
            tree.session_for_frame(&id("F4")),
            Err(FrameLookup::InTransit)
        );
        assert_eq!(tree.local_root(&id("F2")), Err(FrameLookup::InTransit));
        assert!(!tree.in_transit(&id(MAIN)));

        assert!(attached(&mut tree, 5, PAGE, "F2", MAIN));
        assert_eq!(
            tree.resolve_pending_detach(&sid("S2")),
            DetachResolution::SwappedBack
        );
        assert!(!tree.in_transit(&id("F2")));
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid(PAGE)));
        assert_eq!(tree.committed_loader(&id("F2")), None);
        assert!(tree.get(&id("F4")).is_none());
    }

    #[test]
    fn removal_waits_for_the_round_trip_and_is_not_resurrected() {
        let mut tree = page();
        local_child(&mut tree, 2, "F2", "L2");
        attach_child(&mut tree, 4, PAGE, "S2", "F2", MAIN);
        assert!(!detached(&mut tree, 5, PAGE, "F2", "remove"));
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid("S2")));
        assert_eq!(
            tree.on_child_session_detached(&sid("S2")),
            DetachResolution::NeedsRoundTrip
        );
        assert_eq!(
            tree.resolve_pending_detach(&sid("S2")),
            DetachResolution::Removed
        );
        assert!(tree.get(&id("F2")).is_none());
        assert!(tree.get(&id(MAIN)).unwrap().children.is_empty());

        let stale = tree_json(json!({
            "frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p"},
            "childFrames": [{"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L2", "url": "http://a/child"}}]
        }));
        tree.seed(&sid(PAGE), &stale, EventCursor(3));
        assert!(
            tree.get(&id("F2")).is_none(),
            "a reply sent before the removal is stale"
        );
        assert_eq!(
            tree.on_child_session_detached(&sid("S2")),
            DetachResolution::Removed
        );
    }

    #[test]
    fn local_frames_are_removed_with_their_subtree() {
        let mut tree = page();
        local_child(&mut tree, 2, "F2", "L2");
        assert!(attached(&mut tree, 4, PAGE, "F3", "F2"));
        assert!(detached(&mut tree, 5, PAGE, "F2", "remove"));
        assert!(tree.get(&id("F2")).is_none());
        assert!(tree.get(&id("F3")).is_none());
        assert!(tree.descendants_preorder(&id(MAIN)).is_empty());
        assert!(!detached(&mut tree, 6, PAGE, MAIN, "remove"));
        assert!(tree.get(&id(MAIN)).is_some());
    }

    #[test]
    fn stale_parent_session_events_are_ignored() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S2", "F2", MAIN);
        assert!(navigated(
            &mut tree,
            3,
            "S2",
            "F2",
            Some(MAIN),
            "L2",
            "http://b/child"
        ));
        assert!(!navigated(
            &mut tree,
            4,
            PAGE,
            "F2",
            Some(MAIN),
            "L9",
            "http://a/old"
        ));
        assert!(!within_document(&mut tree, 5, PAGE, "F2", "http://a/old#x"));
        assert!(!detached(&mut tree, 6, PAGE, "F2", "swap"));
        assert!(!attached(&mut tree, 7, "S2", "F2", MAIN));
        let node = tree.get(&id("F2")).unwrap();
        assert_eq!(node.loader_id, Some(LoaderId::from("L2")));
        assert_eq!(node.url, "http://b/child");
        assert_eq!(node.last_seq, 3);
        assert_eq!(node.reported_by, sid("S2"));
    }

    #[test]
    fn events_of_unknown_sessions_do_not_invent_frames() {
        let mut tree = page();
        assert!(!navigated(
            &mut tree,
            2,
            "S9",
            "F9",
            Some("F8"),
            "L9",
            "http://x/"
        ));
        assert!(!attached(&mut tree, 3, "S9", "F9", "F8"));
        assert!(tree.get(&id("F9")).is_none());
    }

    #[test]
    fn frames_under_removed_or_moved_parents_are_not_invented() {
        let mut tree = page();
        local_child(&mut tree, 2, "F2", "L2");
        assert!(detached(&mut tree, 4, PAGE, "F2", "remove"));
        assert!(!attached(&mut tree, 5, PAGE, "F5", "F2"));
        assert!(!navigated(
            &mut tree,
            6,
            PAGE,
            "F6",
            Some("F2"),
            "L6",
            "http://a/late"
        ));
        assert!(!navigated(
            &mut tree,
            7,
            PAGE,
            "F8",
            None,
            "L8",
            "http://a/root"
        ));
        attach_child(&mut tree, 8, PAGE, "S3", "F3", MAIN);
        assert!(!attached(&mut tree, 9, PAGE, "F7", "F3"));
        assert!(attached(&mut tree, 10, "S3", "F7", "F3"));
        assert_eq!(tree.session_for_frame(&id("F7")), Ok(sid("S3")));
        for gone in ["F5", "F6", "F8"] {
            assert!(tree.get(&id(gone)).is_none(), "{gone} must not exist");
        }
        assert_eq!(tree.descendants_preorder(&id(MAIN)), ids(&["F3", "F7"]));
    }

    #[test]
    fn main_url_tracks_fragments_and_history_changes() {
        let mut tree = FrameTree::new(id(MAIN), sid(PAGE));
        assert_eq!(tree.main_url(), "", "nothing is committed yet");
        tree.on_event(&event(
            1,
            "Page.frameNavigated",
            PAGE,
            json!({"frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p", "urlFragment": "#a"}, "type": "Navigation"}),
        ));
        assert_eq!(tree.main_url(), "http://a/p#a");
        assert_eq!(tree.get(&id(MAIN)).unwrap().url, "http://a/p");
        assert!(within_document(&mut tree, 2, PAGE, MAIN, "http://a/p#b"));
        assert_eq!(tree.main_url(), "http://a/p#b");
        assert_eq!(
            tree.get(&id(MAIN)).unwrap().url_fragment.as_deref(),
            Some("#b")
        );
        assert!(within_document(&mut tree, 3, PAGE, MAIN, "http://a/q"));
        assert_eq!(tree.main_url(), "http://a/q");
        assert_eq!(tree.get(&id(MAIN)).unwrap().url_fragment, None);
        assert_eq!(tree.committed_loader(&id(MAIN)), Some(LoaderId::from("L1")));
    }

    #[test]
    fn frame_urls_never_keep_a_fragment() {
        let mut tree = page();
        assert!(navigated(
            &mut tree,
            2,
            PAGE,
            MAIN,
            None,
            "L2",
            "http://a/r#s"
        ));
        let node = tree.get(&id(MAIN)).unwrap();
        assert_eq!(node.url, "http://a/r");
        assert_eq!(node.url_fragment.as_deref(), Some("#s"));
        assert_eq!(tree.main_url(), "http://a/r#s");
    }

    #[test]
    fn a_navigation_after_the_tree_request_survives_the_seed() {
        let mut tree = FrameTree::new(id(MAIN), sid(PAGE));
        let cursor = EventCursor(5);
        assert!(navigated(
            &mut tree,
            5,
            PAGE,
            MAIN,
            None,
            "L2",
            "http://a/new"
        ));
        let reply = tree_json(json!({
            "frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/old"},
            "childFrames": [{"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L3", "url": "http://a/old-child"}}]
        }));
        tree.seed(&sid(PAGE), &reply, cursor);
        assert_eq!(tree.committed_loader(&id(MAIN)), Some(LoaderId::from("L2")));
        assert_eq!(tree.main_url(), "http://a/new");
        assert!(
            tree.get(&id("F2")).is_none(),
            "children of the old document stay gone"
        );
    }

    #[test]
    fn a_seed_merges_frames_older_than_its_cursor() {
        let mut tree = page();
        assert!(within_document(&mut tree, 6, PAGE, MAIN, "http://a/p#late"));
        let reply = tree_json(json!({
            "frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p"},
            "childFrames": [
                {"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L2", "url": "http://a/2", "name": "two"}},
                {"frame": {"id": "F3", "parentId": MAIN, "loaderId": "", "url": "http://a/3"}}
            ]
        }));
        tree.seed(&sid(PAGE), &reply, EventCursor(4));
        assert_eq!(tree.main_url(), "http://a/p#late");
        assert_eq!(tree.descendants_preorder(&id(MAIN)), ids(&["F2", "F3"]));
        let two = tree.get(&id("F2")).unwrap();
        assert_eq!(two.name.as_deref(), Some("two"));
        assert_eq!(two.last_seq, 3);
        assert_eq!(
            tree.committed_loader(&id("F3")),
            None,
            "an empty loader is unknown"
        );

        tree.seed(&sid(PAGE), &reply, EventCursor(2));
        assert!(navigated(
            &mut tree,
            7,
            PAGE,
            "F2",
            Some(MAIN),
            "L7",
            "http://a/2b"
        ));
        let newer = tree_json(json!({
            "frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p"},
            "childFrames": [{"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L8", "url": "http://a/2c"}}]
        }));
        tree.seed(&sid(PAGE), &newer, EventCursor(9));
        assert_eq!(tree.committed_loader(&id("F2")), Some(LoaderId::from("L8")));
    }

    #[test]
    fn a_same_document_event_after_the_cursor_keeps_the_first_seed() {
        let mut tree = FrameTree::new(id(MAIN), sid(PAGE));
        assert!(within_document(&mut tree, 5, PAGE, MAIN, "http://a/p#x"));
        let reply = tree_json(json!({
            "frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p"},
            "childFrames": [{"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L2", "url": "http://a/2"}}]
        }));
        tree.seed(&sid(PAGE), &reply, EventCursor(4));
        assert_eq!(tree.committed_loader(&id(MAIN)), Some(LoaderId::from("L1")));
        assert_eq!(tree.main_url(), "http://a/p#x");
        assert_eq!(tree.committed_loader(&id("F2")), Some(LoaderId::from("L2")));
        assert_eq!(tree.get(&id(MAIN)).unwrap().last_seq, 5);
    }

    #[test]
    fn a_same_document_event_on_a_new_oopif_keeps_its_seed() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S2", "F2", MAIN);
        assert!(within_document(&mut tree, 4, "S2", "F2", "http://b/#top"));
        let reply = tree_json(json!({
            "frame": {"id": "F2", "parentId": MAIN, "loaderId": "L5", "url": "http://b/"},
            "childFrames": [{"frame": {"id": "F3", "parentId": "F2", "loaderId": "L6", "url": "http://b/3"}}]
        }));
        tree.seed(&sid("S2"), &reply, EventCursor(3));
        assert_eq!(tree.committed_loader(&id("F2")), Some(LoaderId::from("L5")));
        assert_eq!(
            tree.get(&id("F2")).unwrap().url_fragment.as_deref(),
            Some("#top")
        );
        assert_eq!(tree.frames_of_session(&sid("S2")), ids(&["F2", "F3"]));
    }

    #[test]
    fn a_document_change_after_the_cursor_still_wins_over_the_seed() {
        let mut tree = page();
        attach_child(&mut tree, 4, PAGE, "S2", "F2", MAIN);
        let reply = tree_json(json!({
            "frame": {"id": "F2", "parentId": MAIN, "loaderId": "L5", "url": "http://b/"},
            "childFrames": [{"frame": {"id": "F3", "parentId": "F2", "loaderId": "L6", "url": "http://b/3"}}]
        }));
        tree.seed(&sid("S2"), &reply, EventCursor(3));
        assert_eq!(tree.committed_loader(&id("F2")), None);
        assert!(tree.get(&id("F3")).is_none());
    }

    #[test]
    fn a_placeholder_is_adopted_by_its_parents_seed() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S3", "F3", "F2");
        assert_eq!(tree.session_for_frame(&id("F3")), Ok(sid("S3")));
        assert!(tree.descendants_preorder(&id(MAIN)).is_empty());
        let reply = tree_json(json!({
            "frame": {"id": MAIN, "loaderId": "L1", "url": "http://a/p"},
            "childFrames": [{"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L2", "url": "http://a/2"}}]
        }));
        tree.seed(&sid(PAGE), &reply, EventCursor(1));
        assert_eq!(tree.descendants_preorder(&id(MAIN)), ids(&["F2", "F3"]));
        assert_eq!(tree.session_for_frame(&id("F3")), Ok(sid("S3")));
        assert_eq!(tree.local_root(&id("F3")), Ok(id("F3")));
        assert_eq!(tree.local_root(&id("F2")), Ok(id(MAIN)));
        assert_eq!(tree.frames_of_session(&sid(PAGE)), ids(&[MAIN, "F2"]));
        assert_eq!(tree.committed_loader(&id("F3")), None);
    }

    #[test]
    fn a_seed_of_a_session_that_lost_its_frame_is_dropped() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S2", "F2", MAIN);
        assert!(attached(&mut tree, 3, PAGE, "F2", MAIN));
        let reply = tree_json(
            json!({"frame": {"id": "F2", "parentId": MAIN, "loaderId": "L5", "url": "http://b/"}}),
        );
        tree.seed(&sid("S2"), &reply, EventCursor(4));
        assert_eq!(tree.committed_loader(&id("F2")), None);
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid(PAGE)));
    }

    #[test]
    fn a_new_session_for_the_same_frame_supersedes_the_old_one() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S2", "F2", MAIN);
        assert!(navigated(
            &mut tree,
            3,
            "S2",
            "F2",
            Some(MAIN),
            "L2",
            "http://b/"
        ));
        attach_child(&mut tree, 4, PAGE, "S3", "F2", MAIN);
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid("S3")));
        assert_eq!(tree.committed_loader(&id("F2")), None);
        assert_eq!(
            tree.on_child_session_detached(&sid("S2")),
            DetachResolution::ChildDisposed
        );
        assert!(!tree.in_transit(&id("F2")));
    }

    #[test]
    fn a_superseded_pending_session_resolves_as_disposed() {
        let mut tree = page();
        attach_child(&mut tree, 2, PAGE, "S2", "F2", MAIN);
        assert_eq!(
            tree.on_child_session_detached(&sid("S2")),
            DetachResolution::NeedsRoundTrip
        );
        attach_child(&mut tree, 3, PAGE, "S3", "F2", MAIN);
        assert!(!tree.in_transit(&id("F2")));
        assert_eq!(
            tree.resolve_pending_detach(&sid("S2")),
            DetachResolution::ChildDisposed
        );
        assert_eq!(tree.session_for_frame(&id("F2")), Ok(sid("S3")));
    }

    #[test]
    fn detaching_an_unknown_session_reports_removed() {
        let mut tree = page();
        assert_eq!(
            tree.on_child_session_detached(&sid("S7")),
            DetachResolution::Removed
        );
        assert_eq!(tree.session_for_frame(&id("F7")), Err(FrameLookup::Missing));
        assert_eq!(tree.local_root(&id("F7")), Err(FrameLookup::Missing));
    }

    #[test]
    fn the_main_frame_keeps_the_page_session() {
        let mut tree = page();
        let info = TargetInfo {
            target_id: MAIN.into(),
            ..TargetInfo::default()
        };
        assert!(!tree.on_child_session_attached(&sid(PAGE), &sid("S2"), &info, 2));
        assert!(!attached(&mut tree, 3, "S2", MAIN, "F0"));
        assert_eq!(tree.session_for_frame(&id(MAIN)), Ok(sid(PAGE)));
        assert_eq!(tree.main_id(), &id(MAIN));
    }

    #[test]
    fn a_main_frame_navigation_drops_every_child() {
        let mut tree = page();
        local_child(&mut tree, 2, "F2", "L2");
        attach_child(&mut tree, 4, PAGE, "S3", "F3", MAIN);
        assert!(navigated(
            &mut tree,
            5,
            PAGE,
            MAIN,
            None,
            "L5",
            "http://a/next"
        ));
        assert!(tree.descendants_preorder(&id(MAIN)).is_empty());
        assert_eq!(
            tree.on_child_session_detached(&sid("S3")),
            DetachResolution::Removed
        );
    }

    #[test]
    fn contexts_are_filtered_by_owning_session() {
        let mut contexts = ExecutionContexts::new();
        contexts.on_event(&context("S1", "F2", "old-main", false));
        contexts.on_event(&context("S2", "F2", "new-util", true));
        contexts.on_event(&event(
            1,
            "Runtime.executionContextCreated",
            "S2",
            json!({"context": {"id": 7, "uniqueId": "extension", "name": "other", "auxData": {"frameId": "F2", "isDefault": false}}}),
        ));
        let frame = id("F2");
        let child = sid("S2");
        assert!(contexts.main(&frame, &child).is_none());
        assert_eq!(
            &*contexts.util(&frame, &child).unwrap().unique_id,
            "new-util"
        );
        assert_eq!(
            &*contexts.main(&frame, &sid("S1")).unwrap().unique_id,
            "old-main"
        );
        contexts.mark_atoms_installed("new-util");
        contexts.mark_atoms_installed("unknown");
        assert!(contexts.atoms_installed("new-util"));
        assert!(!contexts.atoms_installed("unknown"));
        contexts.forget_frame(&frame, &child);
        assert!(contexts.main(&frame, &sid("S1")).is_none());
        assert!(contexts.util(&frame, &child).is_some());
        assert!(contexts.on_event(&event(
            2,
            "Runtime.executionContextsCleared",
            "S2",
            json!({})
        )));
        assert!(contexts.util(&frame, &child).is_none());
        assert!(!contexts.atoms_installed("new-util"));
    }

    #[test]
    fn the_newest_context_of_a_world_wins_until_destroyed() {
        let mut contexts = ExecutionContexts::new();
        contexts.on_event(&context(PAGE, MAIN, "first", false));
        contexts.on_event(&context(PAGE, MAIN, "second", false));
        assert_eq!(
            &*contexts.main(&id(MAIN), &sid(PAGE)).unwrap().unique_id,
            "second"
        );
        contexts.mark_atoms_installed("second");
        assert!(contexts.on_event(&event(
            3,
            "Runtime.executionContextDestroyed",
            PAGE,
            json!({"executionContextId": 1, "executionContextUniqueId": "second"}),
        )));
        assert!(!contexts.atoms_installed("second"));
        assert_eq!(
            &*contexts.main(&id(MAIN), &sid(PAGE)).unwrap().unique_id,
            "first"
        );
    }

    #[test]
    fn contexts_of_detached_or_crashed_sessions_are_dropped() {
        let mut contexts = ExecutionContexts::new();
        contexts.on_event(&context(PAGE, MAIN, "page-main", false));
        contexts.on_event(&context("S2", "F2", "child-main", false));
        assert!(contexts.on_event(&event(
            2,
            "Target.detachedFromTarget",
            PAGE,
            json!({"sessionId": "S2", "targetId": "F2"}),
        )));
        assert!(contexts.main(&id("F2"), &sid("S2")).is_none());
        assert!(contexts.main(&id(MAIN), &sid(PAGE)).is_some());
        assert!(contexts.on_event(&event(3, "Inspector.targetCrashed", PAGE, json!({}))));
        assert!(contexts.main(&id(MAIN), &sid(PAGE)).is_none());
    }
}
