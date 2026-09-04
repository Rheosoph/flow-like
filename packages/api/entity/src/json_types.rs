//! JSON-backed column types shared by the generated entities.

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

/// A `Vec<String>` persisted as a JSON array in a `jsonb` column.
///
/// Replaces the former `text[]` columns so the same schema runs on engines
/// without array types. Every call site keeps `Vec<String>` ergonomics through
/// `Deref`/`DerefMut`; membership filtering in SQL uses jsonb containment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[serde(transparent)]
pub struct StringList(pub Vec<String>);

impl StringList {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn into_inner(self) -> Vec<String> {
        self.0
    }
}

impl Deref for StringList {
    type Target = Vec<String>;

    fn deref(&self) -> &Vec<String> {
        &self.0
    }
}

impl DerefMut for StringList {
    fn deref_mut(&mut self) -> &mut Vec<String> {
        &mut self.0
    }
}

impl From<Vec<String>> for StringList {
    fn from(value: Vec<String>) -> Self {
        Self(value)
    }
}

impl From<StringList> for Vec<String> {
    fn from(value: StringList) -> Self {
        value.0
    }
}

impl FromIterator<String> for StringList {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for StringList {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a StringList {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl PartialEq<Vec<String>> for StringList {
    fn eq(&self, other: &Vec<String>) -> bool {
        &self.0 == other
    }
}
