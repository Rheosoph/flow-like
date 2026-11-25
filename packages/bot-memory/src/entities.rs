//! Who and what the facts are about: one entity per person, place, organization, project or
//! thing, holding every name the conversation used for it. A fact is an edge from its subject
//! entity to an optional object entity, labelled by a short relation.
//!
//! Resolution is deterministic and conservative: a mention joins the active entity that shares
//! one of its alias keys, otherwise it becomes a new entity. Names alone never merge two
//! entities by similarity; the owner merges duplicates.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use flow_like_types::create_id;
use serde::{Deserialize, Serialize};

use crate::facts::{Fact, normalize_subject};
use crate::text::{one_line, truncate_chars};

/// The bot's owner, the subject `user`.
pub const OWNER_ENTITY: &str = "e_user";
pub const MAX_ALIASES: usize = 24;
const MAX_NAME_CHARS: usize = 80;
const SPEAKER_PREFIX: &str = "speaker:";
/// Alias keys this short name nothing reliably in free text.
const MIN_MENTION_CHARS: usize = 3;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Person,
    Place,
    Organization,
    Project,
    #[default]
    Thing,
}

impl EntityKind {
    pub const ALL: [Self; 5] = [
        Self::Person,
        Self::Place,
        Self::Organization,
        Self::Project,
        Self::Thing,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Place => "place",
            Self::Organization => "organization",
            Self::Project => "project",
            Self::Thing => "thing",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "people" | "human" => Some(Self::Person),
            "location" | "city" | "country" | "address" => Some(Self::Place),
            "organisation" | "org" | "company" | "team" => Some(Self::Organization),
            other => Self::ALL.into_iter().find(|kind| kind.as_str() == other),
        }
    }

    /// Two specific kinds name different things ("Jordan" the person, "Jordan" the country).
    fn compatible(self, other: Self) -> bool {
        self == other || self == Self::Thing || other == Self::Thing
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub kind: EntityKind,
    /// Alias keys ([`alias_key`]), the name's included.
    pub aliases: Vec<String>,
    /// The conversation named it; a name derived from an old subject string (`user.sister`)
    /// gives way to the first real one.
    pub named: bool,
    /// Merged into another entity by the owner; its facts were moved there.
    pub merged_into: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity {
    pub fn new_id() -> String {
        let id = create_id();
        format!("e_{}", &id[..id.len().min(12)])
    }

    pub fn owner(now: DateTime<Utc>) -> Self {
        Self {
            id: OWNER_ENTITY.to_string(),
            name: "user".to_string(),
            kind: EntityKind::Person,
            aliases: vec!["user".to_string()],
            named: true,
            merged_into: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn is_owner(&self) -> bool {
        self.id == OWNER_ENTITY
    }

    pub fn is_active(&self) -> bool {
        self.merged_into.is_none()
    }

    /// The participant account this entity stands for, from its `speaker:<id>` key.
    pub fn participant(&self) -> Option<&str> {
        self.aliases.iter().find_map(|alias| {
            alias
                .strip_prefix(SPEAKER_PREFIX)
                .filter(|id| !id.contains(':'))
        })
    }

    /// Names worth showing: the aliases that are not the name itself or an internal key.
    pub fn other_names(&self) -> Vec<&str> {
        let name = alias_key(&self.name);
        self.aliases
            .iter()
            .filter(|alias| Some(alias.as_str()) != name.as_deref())
            .filter(|alias| !alias.starts_with(SPEAKER_PREFIX))
            .map(String::as_str)
            .collect()
    }

    /// `[e_k3j2h1g0f9e8] Mia (person; also user.sister)`.
    pub fn line(&self) -> String {
        let mut line = format!("[{}] {} ({}", self.id, self.label(), self.kind.as_str());
        let others = self.other_names();
        if !others.is_empty() {
            line.push_str("; also ");
            line.push_str(&truncate_chars(&others.join(", "), 200));
        }
        line.push(')');
        line
    }

    pub fn label(&self) -> String {
        if self.is_owner() {
            "the user (your owner)".to_string()
        } else {
            self.name.clone()
        }
    }

    fn absorb(&mut self, mention: &Mention, now: DateTime<Utc>) -> bool {
        let mut changed = false;
        for key in &mention.keys {
            if self.aliases.len() >= MAX_ALIASES {
                break;
            }
            if !self.aliases.contains(key) && !(self.is_owner() && key.starts_with("user.")) {
                self.aliases.push(key.clone());
                changed = true;
            }
        }
        if !self.is_owner() {
            if mention.named && !self.named {
                self.name = mention.name.clone();
                self.named = true;
                changed = true;
            }
            if let Some(kind) = mention.kind
                && self.kind == EntityKind::Thing
                && kind != EntityKind::Thing
            {
                self.kind = kind;
                changed = true;
            }
        }
        if changed {
            self.updated_at = now;
        }
        changed
    }
}

/// An entity the extraction declared for a span, with the other names used for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredEntity {
    pub name: String,
    pub kind: EntityKind,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// The canonical form names are matched by: lowercase, without a leading article, relational
/// names as paths from the owner ("my sister", "the user's sister", `user.sister` → `user.sister`).
pub fn alias_key(text: &str) -> Option<String> {
    if !text.chars().any(char::is_alphanumeric) {
        return None;
    }
    let text = text.replace(['’', '`'], "'");
    let base = normalize_subject(&text);
    let mut key = base.as_str();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = key.strip_prefix(article) {
            key = rest;
            break;
        }
    }
    let mut key = key.to_string();
    for prefix in ["user's ", "owner's ", "my "] {
        if let Some(rest) = key.strip_prefix(prefix) {
            key = format!("user.{rest}");
            break;
        }
    }
    let key = key
        .replace("'s ", ".")
        .split('.')
        .map(|part| part.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".");
    (!key.is_empty()).then_some(key)
}

/// Phrases a person would write for a relational key: `user.sister` → "my sister",
/// "user's sister".
fn spoken_forms(key: &str) -> Vec<String> {
    match key.strip_prefix("user.") {
        Some(rest) if !rest.contains('.') => vec![format!("my {rest}"), format!("user's {rest}")],
        Some(_) => Vec::new(),
        None => vec![key.to_string()],
    }
}

/// Whether `phrase` occurs in `text` as whole words (both already lowercase).
fn contains_phrase(text: &str, phrase: &str) -> bool {
    if phrase.chars().count() < MIN_MENTION_CHARS {
        return false;
    }
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    text.match_indices(phrase).any(|(at, _)| {
        boundary(text[..at].chars().next_back())
            && boundary(text[at + phrase.len()..].chars().next())
    })
}

/// The active entities `text` names by one of their aliases, most specific match first. The
/// owner is never matched: every message is about them.
pub fn mentioned_in<'a>(text: &str, entities: &'a [Entity]) -> Vec<&'a Entity> {
    let text = text.replace(['’', '`'], "'").to_lowercase();
    let mut found: Vec<(&Entity, usize)> = entities
        .iter()
        .filter(|entity| entity.is_active() && !entity.is_owner() && entity.participant().is_none())
        .filter_map(|entity| {
            entity
                .aliases
                .iter()
                .flat_map(|alias| spoken_forms(alias))
                .filter(|phrase| contains_phrase(&text, phrase))
                .map(|phrase| phrase.len())
                .max()
                .map(|length| (entity, length))
        })
        .collect();
    found.sort_by(|(a, left), (b, right)| right.cmp(left).then(a.created_at.cmp(&b.created_at)));
    found.into_iter().map(|(entity, _)| entity).collect()
}

/// A name to resolve: a fact's subject or object, or an entity the extraction declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mention {
    name: String,
    kind: Option<EntityKind>,
    keys: BTreeSet<String>,
    named: bool,
    owner: bool,
    participant: bool,
}

impl Mention {
    fn owner() -> Self {
        Self {
            name: "user".to_string(),
            kind: Some(EntityKind::Person),
            keys: BTreeSet::from(["user".to_string()]),
            named: true,
            owner: true,
            participant: false,
        }
    }

    fn participant(id: &str, name: Option<&str>) -> Self {
        Self {
            name: name.map_or_else(|| id.to_string(), |name| clean_name(name)),
            kind: Some(EntityKind::Person),
            keys: BTreeSet::from([format!("{SPEAKER_PREFIX}{id}")]),
            named: name.is_some(),
            owner: false,
            participant: true,
        }
    }

    /// Someone a participant names relative to themselves ("my sister"): never the owner's.
    fn relative(id: &str, name: Option<&str>, relation: &str) -> Self {
        let speaker = name.map_or_else(|| id.to_string(), clean_name);
        Self {
            name: clean_name(&format!("{speaker}'s {}", relation.replace('.', "'s "))),
            kind: None,
            keys: BTreeSet::from([format!("{SPEAKER_PREFIX}{id}:user.{relation}")]),
            named: false,
            owner: false,
            participant: false,
        }
    }

    fn declared(entity: &DeclaredEntity) -> Option<Self> {
        let name = clean_name(&entity.name);
        let mut keys: BTreeSet<String> = std::iter::once(name.as_str())
            .chain(entity.aliases.iter().map(String::as_str))
            .filter_map(alias_key)
            .collect();
        if keys.contains("user") {
            keys.retain(|key| key == "user" || !key.starts_with("user."));
        }
        if keys.is_empty() {
            return None;
        }
        Some(Self {
            owner: keys.contains("user"),
            name,
            kind: Some(entity.kind),
            keys,
            named: true,
            participant: false,
        })
    }

    /// A subject or object string without a declared entity behind it.
    fn plain(text: &str) -> Option<Self> {
        let key = alias_key(text)?;
        if key == "user" {
            return Some(Self::owner());
        }
        Some(Self {
            name: key.clone(),
            kind: None,
            keys: BTreeSet::from([key]),
            named: false,
            owner: false,
            participant: false,
        })
    }
}

fn clean_name(name: &str) -> String {
    truncate_chars(&one_line(name), MAX_NAME_CHARS)
}

/// The mentions an extraction batch makes: its declared entities, by every key they carry.
pub(crate) struct Declared(Vec<Mention>);

impl Declared {
    pub(crate) fn new(entities: &[DeclaredEntity]) -> Self {
        Self(entities.iter().filter_map(Mention::declared).collect())
    }

    fn find(&self, key: &str) -> Option<&Mention> {
        self.0.iter().find(|mention| mention.keys.contains(key))
    }

    /// The mention a subject or object string stands for: a participant, the owner, a
    /// declared entity it names, or the string alone.
    fn mention(&self, text: &str, fact: &Fact) -> Option<Mention> {
        if let Some(rest) = text.strip_prefix(SPEAKER_PREFIX)
            && let Some((id, inner)) = rest.split_once(':')
        {
            let name = fact
                .evidence
                .iter()
                .filter_map(|evidence| evidence.speaker.as_ref())
                .find(|speaker| speaker.id == id)
                .and_then(|speaker| speaker.name.as_deref());
            let key = alias_key(inner)?;
            if key == "user" {
                return Some(Mention::participant(id, name));
            }
            if let Some(relation) = key.strip_prefix("user.") {
                return Some(Mention::relative(id, name, relation));
            }
            return self.mention(inner, fact);
        }
        let key = alias_key(text)?;
        match self.find(&key) {
            Some(declared) => Some(declared.clone()),
            None => Mention::plain(text),
        }
    }
}

/// Resolves mentions against the stored entities and collects the entities to write. Changes
/// stay in memory until the caller writes [`Resolver::writes`] in its commit.
pub(crate) struct Resolver {
    entities: Vec<Entity>,
    by_key: BTreeMap<String, Vec<usize>>,
    by_id: BTreeMap<String, usize>,
    dirty: BTreeSet<usize>,
    now: DateTime<Utc>,
}

impl Resolver {
    pub(crate) fn new(entities: Vec<Entity>, now: DateTime<Utc>) -> Self {
        let mut resolver = Self {
            entities: Vec::new(),
            by_key: BTreeMap::new(),
            by_id: BTreeMap::new(),
            dirty: BTreeSet::new(),
            now,
        };
        for entity in entities {
            resolver.push(entity);
        }
        resolver
    }

    fn push(&mut self, entity: Entity) -> usize {
        let index = self.entities.len();
        self.by_id.insert(entity.id.clone(), index);
        if entity.is_active() {
            for key in &entity.aliases {
                self.by_key.entry(key.clone()).or_default().push(index);
            }
        }
        self.entities.push(entity);
        index
    }

    fn index_keys(&mut self, index: usize) {
        for key in self.entities[index].aliases.clone() {
            let indexes = self.by_key.entry(key).or_default();
            if !indexes.contains(&index) {
                indexes.push(index);
            }
        }
    }

    /// The live entity an id names, following merges.
    pub(crate) fn live(&self, id: &str) -> Option<&Entity> {
        let mut entity = self.entities.get(*self.by_id.get(id)?)?;
        for _ in 0..8 {
            match &entity.merged_into {
                Some(target) => entity = self.entities.get(*self.by_id.get(target)?)?,
                None => return Some(entity),
            }
        }
        None
    }

    fn resolve(&mut self, mention: &Mention) -> String {
        match self.find_or_absorb(mention) {
            Some(id) => id,
            None => {
                let index = self.push(Entity {
                    id: Entity::new_id(),
                    name: mention.name.clone(),
                    kind: mention.kind.unwrap_or_default(),
                    aliases: mention.keys.iter().take(MAX_ALIASES).cloned().collect(),
                    named: mention.named,
                    merged_into: None,
                    created_at: self.now,
                    updated_at: self.now,
                });
                self.dirty.insert(index);
                self.entities[index].id.clone()
            }
        }
    }

    /// The entity a mention names, which takes the mention's new names; `None` when no stored
    /// entity matches.
    fn find_or_absorb(&mut self, mention: &Mention) -> Option<String> {
        if mention.owner {
            let index = match self.by_id.get(OWNER_ENTITY) {
                Some(index) => *index,
                None => {
                    let index = self.push(Entity::owner(self.now));
                    self.dirty.insert(index);
                    index
                }
            };
            if self.entities[index].absorb(mention, self.now) {
                self.dirty.insert(index);
                self.index_keys(index);
            }
            return Some(self.entities[index].id.clone());
        }
        let kind = mention.kind.unwrap_or_default();
        let mut best: Option<(usize, usize)> = None;
        for key in &mention.keys {
            for &index in self.by_key.get(key).into_iter().flatten() {
                let entity = &self.entities[index];
                if !entity.is_active()
                    || !entity.kind.compatible(kind)
                    || entity.participant().is_some() != mention.participant
                {
                    continue;
                }
                let overlap = mention
                    .keys
                    .iter()
                    .filter(|key| entity.aliases.contains(key))
                    .count();
                let better = best.is_none_or(|(known, known_overlap)| {
                    overlap > known_overlap
                        || (overlap == known_overlap
                            && (entity.created_at, &entity.id)
                                < (self.entities[known].created_at, &self.entities[known].id))
                });
                if better {
                    best = Some((index, overlap));
                }
            }
        }
        let (index, _) = best?;
        if self.entities[index].absorb(mention, self.now) {
            self.dirty.insert(index);
            self.index_keys(index);
        }
        Some(self.entities[index].id.clone())
    }

    /// Points `fact`'s subject and object at entities. Every declared entity is resolved too,
    /// so names the facts don't use still join their entity.
    pub(crate) fn attach(&mut self, fact: &mut Fact, declared: &Declared) {
        if let Some(mention) = declared.mention(&fact.subject, fact) {
            fact.subject_id = Some(self.resolve(&mention));
        }
        fact.object_id = fact
            .object
            .clone()
            .and_then(|object| declared.mention(&object, fact))
            .map(|mention| self.resolve(&mention))
            .filter(|id| fact.subject_id.as_ref() != Some(id));
    }

    /// Lets stored entities take the names a batch declared for them. A declared entity no
    /// fact uses creates nothing.
    pub(crate) fn declare(&mut self, declared: &Declared) {
        for mention in declared.0.clone() {
            self.find_or_absorb(&mention);
        }
    }

    /// The entity for a subject string alone, as maintenance backfills old facts.
    pub(crate) fn subject(&mut self, subject: &str, fact: &Fact) -> Option<String> {
        Declared(Vec::new())
            .mention(subject, fact)
            .map(|mention| self.resolve(&mention))
    }

    /// The alias keys a subject scope covers, for facts stored before entities existed.
    pub(crate) fn keys(&self, id: &str) -> Vec<String> {
        self.live(id)
            .map(|entity| entity.aliases.clone())
            .unwrap_or_default()
    }

    pub(crate) fn writes(&self) -> Vec<Entity> {
        self.dirty
            .iter()
            .map(|index| self.entities[*index].clone())
            .collect()
    }
}

/// Facts whose subject belongs to a participant keep their exact subject as their update
/// scope: participant claims never share the owner's namespace.
pub fn is_participant_subject(subject: &str) -> bool {
    subject.starts_with(SPEAKER_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::FactKind;
    use crate::model::{FactSource, Speaker};

    fn fact(subject: &str, object: Option<&str>) -> Fact {
        let mut fact = Fact {
            id: Fact::new_id(),
            text: format!("{subject} fact"),
            kind: FactKind::Knowledge,
            subject: subject.to_string(),
            subject_id: None,
            object: object.map(str::to_string),
            object_id: None,
            relation: None,
            geojson: None,
            time: None,
            expires_at: None,
            valid_from: None,
            valid_to: None,
            recorded_at: Utc::now(),
            expired_at: None,
            evidence: Vec::new(),
            segment_id: None,
            source: FactSource::UserStated,
            confidence: 0.9,
            superseded_by: None,
            content_hash: String::new(),
        };
        fact.rehash();
        fact
    }

    fn declared(name: &str, kind: EntityKind, aliases: &[&str]) -> DeclaredEntity {
        DeclaredEntity {
            name: name.to_string(),
            kind,
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
        }
    }

    #[test]
    fn relational_names_share_one_key() {
        for name in [
            "user.sister",
            "my sister",
            "The user's sister",
            "user’s sister",
            "owner's  sister",
        ] {
            assert_eq!(alias_key(name).as_deref(), Some("user.sister"), "{name}");
        }
        assert_eq!(
            alias_key("the user's sister's husband").as_deref(),
            Some("user.sister.husband")
        );
        assert_eq!(alias_key("Project Atlas").as_deref(), Some("project atlas"));
        assert_eq!(alias_key("I").as_deref(), Some("user"));
        assert_eq!(alias_key("  "), None);
    }

    #[test]
    fn a_declared_name_joins_the_legacy_subject_and_renames_it() {
        let now = Utc::now();
        let mut resolver = Resolver::new(Vec::new(), now);
        let mut old = fact("user.sister", None);
        resolver.attach(&mut old, &Declared::new(&[]));
        let sister = old.subject_id.clone().unwrap();
        assert_eq!(resolver.live(&sister).unwrap().name, "user.sister");

        let declared =
            Declared::new(&[declared("Mia", EntityKind::Person, &["the user's sister"])]);
        let mut new = fact("mia", Some("lyon"));
        resolver.attach(&mut new, &declared);
        assert_eq!(new.subject_id.as_deref(), Some(sister.as_str()));
        let mia = resolver.live(&sister).unwrap();
        assert_eq!(mia.name, "Mia");
        assert_eq!(mia.kind, EntityKind::Person);
        assert!(mia.aliases.contains(&"mia".to_string()));
        let lyon = new.object_id.clone().unwrap();
        assert_ne!(lyon, sister);
        assert_eq!(resolver.live(&lyon).unwrap().name, "lyon");
    }

    #[test]
    fn different_specific_kinds_never_merge() {
        let now = Utc::now();
        let mut resolver = Resolver::new(Vec::new(), now);
        let mut person = fact("jordan", None);
        resolver.attach(
            &mut person,
            &Declared::new(&[declared("Jordan", EntityKind::Person, &[])]),
        );
        let mut place = fact("jordan", None);
        resolver.attach(
            &mut place,
            &Declared::new(&[declared("Jordan", EntityKind::Place, &[])]),
        );
        assert_ne!(person.subject_id, place.subject_id);
        assert_eq!(resolver.writes().len(), 2);
    }

    #[test]
    fn declared_names_without_facts_only_enrich_stored_entities() {
        let mut resolver = Resolver::new(Vec::new(), Utc::now());
        let mut mia = fact("mia", None);
        resolver.attach(&mut mia, &Declared::new(&[]));
        resolver.declare(&Declared::new(&[
            declared("Mia", EntityKind::Person, &["user's sister"]),
            declared("Lena", EntityKind::Person, &[]),
        ]));
        let writes = resolver.writes();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].aliases.contains(&"user.sister".to_string()));
        assert_eq!(writes[0].kind, EntityKind::Person);
    }

    #[test]
    fn a_participants_relatives_are_not_the_owners() {
        let mut resolver = Resolver::new(Vec::new(), Utc::now());
        let mut owner_sister = fact("user.sister", None);
        resolver.attach(&mut owner_sister, &Declared::new(&[]));
        let mut claim = fact("speaker:acct_1:my sister", None);
        resolver.attach(&mut claim, &Declared::new(&[]));
        assert_ne!(claim.subject_id, owner_sister.subject_id);
        let relative = resolver.live(claim.subject_id.as_ref().unwrap()).unwrap();
        assert_eq!(relative.name, "acct_1's sister");
        assert_eq!(relative.participant(), None);
    }

    #[test]
    fn the_owner_and_participants_resolve_to_their_own_entities() {
        let now = Utc::now();
        let mut resolver = Resolver::new(Vec::new(), now);
        let mut own = fact("user", None);
        resolver.attach(&mut own, &Declared::new(&[]));
        assert_eq!(own.subject_id.as_deref(), Some(OWNER_ENTITY));

        let mut claim = fact("speaker:acct_1:user", Some("speaker:acct_1:berlin"));
        claim.evidence.push(crate::facts::Evidence {
            seq: 3,
            message_id: "m3".into(),
            speaker: Some(Speaker::new("acct_1").with_name("Ana")),
            part: None,
            quote: "I live in Berlin".into(),
            at: now,
            partial: false,
        });
        resolver.attach(&mut claim, &Declared::new(&[]));
        let participant = resolver.live(claim.subject_id.as_ref().unwrap()).unwrap();
        assert_eq!(participant.name, "Ana");
        assert_eq!(participant.participant(), Some("acct_1"));
        assert_ne!(participant.id, OWNER_ENTITY);
        assert_eq!(
            resolver
                .live(claim.object_id.as_ref().unwrap())
                .unwrap()
                .name,
            "berlin"
        );

        let mut ana = fact("ana", None);
        resolver.attach(&mut ana, &Declared::new(&[]));
        assert_ne!(
            ana.subject_id, claim.subject_id,
            "names never join a participant"
        );
    }

    #[test]
    fn the_owner_never_takes_relational_aliases() {
        let now = Utc::now();
        let mut resolver = Resolver::new(Vec::new(), now);
        resolver.declare(&Declared::new(&[declared(
            "Felix",
            EntityKind::Person,
            &["user", "user.sister"],
        )]));
        let owner = resolver.live(OWNER_ENTITY).unwrap();
        assert_eq!(owner.name, "user");
        assert!(owner.aliases.contains(&"felix".to_string()));
        assert!(!owner.aliases.iter().any(|alias| alias.starts_with("user.")));
    }

    #[test]
    fn a_fact_never_points_at_its_own_subject_as_object() {
        let mut resolver = Resolver::new(Vec::new(), Utc::now());
        let mut fact = fact("mia", Some("Mia"));
        resolver.attach(&mut fact, &Declared::new(&[]));
        assert!(fact.subject_id.is_some());
        assert_eq!(fact.object_id, None);
    }

    #[test]
    fn mentions_are_whole_words_and_skip_the_owner() {
        let now = Utc::now();
        let mut resolver = Resolver::new(Vec::new(), now);
        let declared = Declared::new(&[
            declared("Mia", EntityKind::Person, &["user.sister"]),
            declared("Al", EntityKind::Person, &[]),
            declared("Acme", EntityKind::Organization, &[]),
        ]);
        for subject in ["mia", "al", "acme", "user"] {
            resolver.attach(&mut fact(subject, None), &declared);
        }
        let entities = resolver.writes();
        let names = |text: &str| -> Vec<String> {
            mentioned_in(text, &entities)
                .into_iter()
                .map(|entity| entity.name.clone())
                .collect()
        };
        assert_eq!(names("What does my sister like?"), ["Mia"]);
        assert_eq!(names("Ask MIA and Acme."), ["Acme", "Mia"]);
        assert!(names("Miami and acmeist").is_empty());
        assert!(names("Al said hi to the user").is_empty());
    }
}
