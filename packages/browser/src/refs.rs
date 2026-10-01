use std::collections::{BTreeMap, HashMap, HashSet};

use crate::element::Element;
use crate::error::{BrowserError, ErrorClass};
use crate::page::{Frame, Page};
use crate::script::World;
use crate::settle::OpSpec;
use crate::snapshot::page_session_gone;
use crate::types::{FrameId, LoaderId, SessionId, TargetId};

pub const MAX_REF_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeRef {
    pub page: TargetId,
    pub local_root: TargetId,
    pub frame_id: FrameId,
    pub loader_id: LoaderId,
    pub backend_node_id: i64,
}

#[derive(Clone, Debug)]
pub struct RefEntry {
    pub node: NodeRef,
    pub role: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct RefTable {
    generation: u64,
    page: Option<TargetId>,
    main_loader: Option<LoaderId>,
    entries: BTreeMap<u64, RefEntry>,
    latest_by_node: HashMap<NodeRef, u64>,
    last_ref: u64,
}

fn ref_label(number: u64) -> String {
    format!("e{number}")
}

fn ref_number(reference: &str) -> Option<u64> {
    let digits = reference.strip_prefix('e')?;
    let canonical = !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit());
    if !canonical {
        return None;
    }
    digits.parse().ok()
}

fn stale_ref(reference: &str) -> BrowserError {
    BrowserError::StaleRef {
        reference: reference.to_owned(),
    }
}

impl RefTable {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn page(&self) -> Option<&TargetId> {
        self.page.as_ref()
    }

    pub fn main_loader(&self) -> Option<&LoaderId> {
        self.main_loader.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn lookup(&self, reference: &str) -> crate::Result<&RefEntry> {
        ref_number(reference)
            .and_then(|number| self.entries.get(&number))
            .ok_or_else(|| stale_ref(reference))
    }

    pub fn register(&mut self, node: NodeRef) -> String {
        if let Some(number) = self.latest_by_node.get(&node) {
            return ref_label(*number);
        }
        let number = self.last_ref + 1;
        self.insert(
            number,
            RefEntry {
                node,
                role: String::new(),
                name: String::new(),
            },
        );
        ref_label(number)
    }

    pub fn begin(&self, page: &TargetId, main_loader: &LoaderId) -> RefAllocator {
        let same_document =
            self.page.as_ref() == Some(page) && self.main_loader.as_ref() == Some(main_loader);
        let table = if same_document {
            self.clone()
        } else {
            RefTable {
                page: Some(page.clone()),
                main_loader: Some(main_loader.clone()),
                ..RefTable::default()
            }
        };
        RefAllocator {
            generation: self.generation + 1,
            previous: table.latest_by_node.clone(),
            committed: HashSet::new(),
            table,
        }
    }

    fn insert(&mut self, number: u64, entry: RefEntry) {
        let latest = self.latest_by_node.entry(entry.node.clone()).or_default();
        *latest = (*latest).max(number);
        self.last_ref = self.last_ref.max(number);
        self.entries.insert(number, entry);
    }

    fn retain_only(&mut self, keep: &HashSet<u64>) {
        self.entries.retain(|number, _| keep.contains(number));
        self.latest_by_node = self
            .entries
            .iter()
            .map(|(number, entry)| (entry.node.clone(), *number))
            .collect();
    }
}

pub struct RefAllocator {
    generation: u64,
    table: RefTable,
    previous: HashMap<NodeRef, u64>,
    committed: HashSet<u64>,
}

#[derive(Clone, Debug)]
pub struct ProposedRef {
    pub reference: String,
    pub fresh: bool,
}

impl RefAllocator {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn propose(&self, node: &NodeRef, role: &str, name: &str) -> ProposedRef {
        let previous = self
            .previous
            .get(node)
            .filter(|number| {
                self.table
                    .entries
                    .get(*number)
                    .is_some_and(|entry| entry.role == role && entry.name == name)
            })
            .copied();
        match previous {
            Some(number) => ProposedRef {
                reference: ref_label(number),
                fresh: false,
            },
            None => ProposedRef {
                reference: ref_label(self.table.last_ref + 1),
                fresh: true,
            },
        }
    }

    pub fn commit(&mut self, proposed: ProposedRef, node: NodeRef, role: String, name: String) {
        let Some(number) = ref_number(&proposed.reference) else {
            tracing::warn!(
                target: "flow_like_browser::refs",
                reference = %proposed.reference,
                "a proposed ref is not of the form eN and was not stored"
            );
            return;
        };
        self.committed.insert(number);
        self.table.insert(number, RefEntry { node, role, name });
    }

    pub fn finish(self) -> RefTable {
        let mut table = self.table;
        table.generation = self.generation;
        if table.entries.len() > MAX_REF_ENTRIES {
            table.retain_only(&self.committed);
        }
        table
    }
}

fn stale_if_gone(error: BrowserError, reference: &str) -> BrowserError {
    if error.class() == ErrorClass::NodeGone {
        stale_ref(reference)
    } else {
        error
    }
}

fn went_stale(
    page: &Page,
    session: &SessionId,
    error: &BrowserError,
    recheck: impl FnOnce() -> bool,
) -> bool {
    match (error, error.class()) {
        (BrowserError::NoSuchFrame { .. }, _)
        | (_, ErrorClass::NodeGone | ErrorClass::FrameInTransit) => true,
        (_, ErrorClass::SessionGone) => *session != page.inner.session && !page_session_gone(page),
        (BrowserError::DialogOpen { .. }, _) | (_, ErrorClass::Fatal | ErrorClass::Timeout) => {
            false
        }
        _ => recheck(),
    }
}

impl Page {
    pub async fn resolve_node(
        &self,
        node: &NodeRef,
        reference: &str,
        table_main_loader: Option<&LoaderId>,
    ) -> crate::Result<crate::element::Element> {
        let main = self.main_frame();
        let referenced = || referenced_element(self, node, reference, table_main_loader);
        self.run_op(main.id(), OpSpec::READ, |attempt| async move {
            let (element, session) = referenced()?;
            match element.resolve_in(&attempt, World::Util).await {
                Ok(_) => Ok(element),
                Err(error) if went_stale(self, &session, &error, || referenced().is_err()) => {
                    Err(stale_ref(reference))
                }
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|error| stale_if_gone(error, reference))?
        .read()
    }

    pub fn main_loader(&self) -> Option<LoaderId> {
        let state = self.inner.lock_state();
        state.frames.committed_loader(state.frames.main_id())
    }
}

fn referenced_element(
    page: &Page,
    node: &NodeRef,
    reference: &str,
    table_main_loader: Option<&LoaderId>,
) -> crate::Result<(Element, SessionId)> {
    if node.page != *page.target_id() || table_main_loader != page.main_loader().as_ref() {
        return Err(stale_ref(reference));
    }
    let frame = Frame {
        page: page.clone(),
        id: node.frame_id.clone(),
    };
    let stamp = frame.stamp().map_err(|_| stale_ref(reference))?;
    if stamp.loader != node.loader_id || stamp.local_root != node.local_root {
        return Err(stale_ref(reference));
    }
    let session = stamp.session.clone();
    let element = Element::new(frame, &stamp, node.backend_node_id, Some(reference.into()));
    Ok((element, session))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STALE_E7: &str = "Stale element ref 'e7' — take a new browser snapshot";

    fn node(frame: &str, loader: &str, backend_node_id: i64) -> NodeRef {
        NodeRef {
            page: TargetId::from("T1"),
            local_root: TargetId::from(frame),
            frame_id: FrameId::from(frame),
            loader_id: LoaderId::from(loader),
            backend_node_id,
        }
    }

    fn snapshot(table: &RefTable, loader: &str, nodes: &[(NodeRef, &str, &str)]) -> RefTable {
        let mut allocator = table.begin(&TargetId::from("T1"), &LoaderId::from(loader));
        for (node, role, name) in nodes {
            let proposed = allocator.propose(node, role, name);
            allocator.commit(proposed, node.clone(), role.to_string(), name.to_string());
        }
        allocator.finish()
    }

    fn reference_of(table: &RefTable, node: &NodeRef) -> String {
        table
            .entries
            .iter()
            .rev()
            .find(|(_, entry)| entry.node == *node)
            .map(|(number, _)| ref_label(*number))
            .unwrap_or_else(|| panic!("node {} has no ref", node.backend_node_id))
    }

    #[test]
    fn refs_start_at_e1_and_a_new_table_is_generation_one() {
        let empty = RefTable::default();
        assert!(empty.is_empty());
        assert_eq!(empty.generation(), 0);
        assert_eq!(empty.page(), None);
        let button = node("T1", "L1", 5);
        let link = node("T1", "L1", 6);
        let table = snapshot(
            &empty,
            "L1",
            &[
                (button.clone(), "button", "OK"),
                (link.clone(), "link", "Home"),
            ],
        );
        assert_eq!(table.generation(), 1);
        assert_eq!(table.page(), Some(&TargetId::from("T1")));
        assert_eq!(table.main_loader(), Some(&LoaderId::from("L1")));
        assert_eq!(table.lookup("e1").unwrap().node, button);
        assert_eq!(table.lookup("e2").unwrap().node, link);
        assert_eq!(table.lookup("e2").unwrap().role, "link");
        assert_eq!(table.lookup("e2").unwrap().name, "Home");
    }

    #[test]
    fn refs_carry_over_and_are_reused_while_role_and_name_match() {
        let button = node("T1", "L1", 5);
        let link = node("T1", "L1", 6);
        let first = snapshot(
            &RefTable::default(),
            "L1",
            &[
                (button.clone(), "button", "OK"),
                (link.clone(), "link", "Home"),
            ],
        );
        let heading = node("T1", "L1", 9);
        let second = snapshot(
            &first,
            "L1",
            &[
                (button.clone(), "button", "Cancel"),
                (link.clone(), "link", "Home"),
                (heading.clone(), "heading", "Title"),
            ],
        );
        assert_eq!(second.generation(), 2);
        assert_eq!(reference_of(&second, &link), "e2");
        assert_eq!(reference_of(&second, &button), "e3");
        assert_eq!(reference_of(&second, &heading), "e4");
        assert_eq!(second.lookup("e1").unwrap().name, "OK");
        assert_eq!(second.lookup("e3").unwrap().name, "Cancel");
        let third = snapshot(&second, "L1", &[(button.clone(), "button", "Cancel")]);
        assert_eq!(reference_of(&third, &button), "e3");
        assert_eq!(third.generation(), 3);
    }

    #[test]
    fn the_highest_numbered_ref_of_a_node_is_the_one_reused() {
        let button = node("T1", "L1", 5);
        let first = snapshot(
            &RefTable::default(),
            "L1",
            &[(button.clone(), "button", "OK")],
        );
        let second = snapshot(&first, "L1", &[(button.clone(), "button", "Cancel")]);
        let mut allocator = second.begin(&TargetId::from("T1"), &LoaderId::from("L1"));
        let reused = allocator.propose(&button, "button", "Cancel");
        assert_eq!(reused.reference, "e2");
        assert!(!reused.fresh);
        let renamed = allocator.propose(&button, "button", "OK");
        assert_eq!(
            renamed.reference, "e3",
            "only the latest ref is a reuse candidate"
        );
        assert!(renamed.fresh);
        allocator.commit(reused, button.clone(), "button".into(), "Cancel".into());
        assert_eq!(allocator.finish().last_ref, 2);
    }

    #[test]
    fn a_new_main_document_or_page_starts_over() {
        let button = node("T1", "L1", 5);
        let first = snapshot(
            &RefTable::default(),
            "L1",
            &[(button.clone(), "button", "OK")],
        );
        let reloaded = snapshot(&first, "L2", &[(node("T1", "L2", 5), "button", "OK")]);
        assert_eq!(reloaded.generation(), 2);
        assert_eq!(reloaded.main_loader(), Some(&LoaderId::from("L2")));
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(
            reloaded.lookup("e1").unwrap().node.loader_id,
            LoaderId::from("L2")
        );
        let other_page = reloaded.begin(&TargetId::from("T9"), &LoaderId::from("L2"));
        assert_eq!(other_page.generation(), 3);
        assert!(other_page.table.is_empty());
        assert_eq!(other_page.propose(&button, "button", "OK").reference, "e1");
    }

    #[test]
    fn same_ids_in_other_frames_get_their_own_refs() {
        let main = node("T1", "L1", 5);
        let oopif = node("F2", "LF2", 5);
        let table = snapshot(
            &RefTable::default(),
            "L1",
            &[
                (main.clone(), "button", "OK"),
                (oopif.clone(), "button", "OK"),
            ],
        );
        assert_eq!(reference_of(&table, &main), "e1");
        assert_eq!(reference_of(&table, &oopif), "e2");
    }

    #[test]
    fn a_proposal_consumes_no_number_until_it_is_committed() {
        let allocator = RefTable::default().begin(&TargetId::from("T1"), &LoaderId::from("L1"));
        let first = allocator.propose(&node("T1", "L1", 1), "button", "A");
        let second = allocator.propose(&node("T1", "L1", 2), "button", "B");
        assert_eq!(first.reference, "e1");
        assert_eq!(second.reference, "e1");
        assert!(allocator.finish().is_empty());
    }

    #[test]
    fn oversized_tables_keep_only_the_refs_of_the_latest_snapshot() {
        let nodes: Vec<(NodeRef, &str, &str)> = (0..MAX_REF_ENTRIES as i64)
            .map(|id| (node("T1", "L1", id), "button", "A"))
            .collect();
        let full = snapshot(&RefTable::default(), "L1", &nodes);
        assert_eq!(full.entries.len(), MAX_REF_ENTRIES);
        let renamed: Vec<(NodeRef, &str, &str)> = (0..3)
            .map(|id| (node("T1", "L1", id), "button", "B"))
            .collect();
        let trimmed = snapshot(&full, "L1", &renamed);
        assert_eq!(trimmed.entries.len(), 3);
        let kept: Vec<String> = trimmed
            .entries
            .keys()
            .map(|number| ref_label(*number))
            .collect();
        let first_new = MAX_REF_ENTRIES as u64 + 1;
        assert_eq!(
            kept,
            (first_new..first_new + 3)
                .map(ref_label)
                .collect::<Vec<_>>()
        );
        assert!(trimmed.lookup("e1").is_err());
        let next = trimmed
            .begin(&TargetId::from("T1"), &LoaderId::from("L1"))
            .propose(&node("T1", "L1", 99_999), "button", "C");
        assert_eq!(next.reference, ref_label(first_new + 3));
        assert_eq!(
            reference_of(&trimmed, &node("T1", "L1", 0)),
            ref_label(first_new)
        );
    }

    #[test]
    fn tables_at_the_limit_are_not_trimmed() {
        let nodes: Vec<(NodeRef, &str, &str)> = (0..MAX_REF_ENTRIES as i64 - 1)
            .map(|id| (node("T1", "L1", id), "button", "A"))
            .collect();
        let full = snapshot(&RefTable::default(), "L1", &nodes);
        let grown = snapshot(&full, "L1", &[(node("T1", "L1", -1), "button", "A")]);
        assert_eq!(grown.entries.len(), MAX_REF_ENTRIES);
        assert!(grown.lookup("e1").is_ok());
    }

    #[test]
    fn unknown_refs_are_stale_with_the_verbatim_text() {
        let table = snapshot(
            &RefTable::default(),
            "L1",
            &[(node("T1", "L1", 5), "button", "OK")],
        );
        for reference in ["e7", "e0", "e01", "E1", "1", "e", "e1x", "e+1"] {
            let error = table.lookup(reference).unwrap_err();
            assert!(
                matches!(&error, BrowserError::StaleRef { reference: stale } if stale == reference),
                "{reference}: {error:?}"
            );
        }
        assert_eq!(table.lookup("e7").unwrap_err().to_string(), STALE_E7);
        assert_eq!(
            RefTable::default().lookup("e7").unwrap_err().to_string(),
            STALE_E7
        );
    }

    #[test]
    fn register_reuses_the_latest_ref_or_allocates_an_unnamed_one() {
        let button = node("T1", "L1", 5);
        let mut table = snapshot(
            &RefTable::default(),
            "L1",
            &[(button.clone(), "button", "OK")],
        );
        table = snapshot(&table, "L1", &[(button.clone(), "button", "Cancel")]);
        assert_eq!(table.register(button.clone()), "e2");
        let listed = node("T1", "L1", 40);
        assert_eq!(table.register(listed.clone()), "e3");
        assert_eq!(table.register(listed.clone()), "e3");
        let entry = table.lookup("e3").unwrap();
        assert_eq!(entry.node, listed);
        assert!(entry.role.is_empty() && entry.name.is_empty());
        let next = table.begin(&TargetId::from("T1"), &LoaderId::from("L1"));
        assert_eq!(
            next.propose(&node("T1", "L1", 41), "link", "x").reference,
            "e4"
        );
        let mut unsnapshotted = RefTable::default();
        assert_eq!(unsnapshotted.register(listed), "e1");
    }
}
