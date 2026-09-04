//! Relations the dense-format codegen does not emit.
//!
//! `sea-orm-codegen` collapses a pure junction table into a single many-to-many
//! relation, so the path -> junction link disappears. Callers that need the
//! junction's own columns (`position`, `course_id`) have to traverse it directly.

use sea_orm::{Related, RelationDef, RelationTrait};

use crate::{learning_path, learning_path_course};

impl Related<learning_path_course::Entity> for learning_path::Entity {
    fn to() -> RelationDef {
        learning_path_course::Relation::LearningPath.def().rev()
    }
}
