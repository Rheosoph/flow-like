pub struct Person {
    pub id: i64,
    pub name: &'static str,
    pub score: f64,
    pub active: bool,
    pub created_at: &'static str,
}

/// Every backend is seeded with these rows so one set of assertions covers them all.
/// The non-ASCII name catches charset loss on the wire.
pub const PEOPLE: [Person; 5] = [
    Person {
        id: 1,
        name: "ada",
        score: 91.5,
        active: true,
        created_at: "2026-01-01 08:00:00",
    },
    Person {
        id: 2,
        name: "grace",
        score: 78.0,
        active: false,
        created_at: "2026-01-02 09:30:00",
    },
    Person {
        id: 3,
        name: "linus",
        score: 85.25,
        active: true,
        created_at: "2026-01-03 10:15:00",
    },
    Person {
        id: 4,
        name: "margaret",
        score: 62.0,
        active: true,
        created_at: "2026-01-04 11:45:00",
    },
    Person {
        id: 5,
        name: "zoë",
        score: 99.0,
        active: false,
        created_at: "2026-01-05 12:00:00",
    },
];

/// `PEOPLE` as a SQL `VALUES` list in a backend's literal syntax.
pub fn values_sql(boolean: fn(bool) -> String, timestamp: fn(&str) -> String) -> String {
    PEOPLE
        .iter()
        .map(|person| {
            format!(
                "({}, '{}', {:?}, {}, {})",
                person.id,
                person.name,
                person.score,
                boolean(person.active),
                timestamp(person.created_at)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn ansi_boolean(value: bool) -> String {
    if value { "TRUE" } else { "FALSE" }.to_string()
}

pub fn numeric_boolean(value: bool) -> String {
    if value { "1" } else { "0" }.to_string()
}

pub fn ansi_timestamp(value: &str) -> String {
    format!("TIMESTAMP '{value}'")
}

pub fn quoted_timestamp(value: &str) -> String {
    format!("'{value}'")
}
