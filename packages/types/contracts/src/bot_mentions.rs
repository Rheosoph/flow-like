//! A bot names another room member in its text as `@Name`, the way a person picks a name from the
//! composer's menu. Hosts turn those names into `mentions`, so a bot's `@Name` addresses the
//! member it names just as a person's does.

use crate::bot_workspace_links::code_ranges;
use std::collections::BTreeMap;

fn word_char_before(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '@' | '/' | '\\')
}

fn word_char_after(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '/' | '-')
}

fn folded(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().flat_map(char::to_lowercase)
}

/// The byte length of the prefix of `text` that is `name` in any case, if `text` starts with it.
fn prefix_len(text: &str, name: &str) -> Option<usize> {
    let count = name.chars().count();
    let end = text
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(at, _)| at);
    let prefix = &text[..end];
    (prefix.chars().count() == count && folded(prefix).eq(folded(name))).then_some(end)
}

/// The ids among `members` (id, display name) whose `@Name` stands in `text` as a whole word,
/// outside code, in any case, in the order the text first names them. The longest name wins
/// where names overlap; a name several members share names none of them.
pub fn named_mentions<'a>(
    text: &str,
    members: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<String> {
    if !text.contains('@') {
        return Vec::new();
    }
    let mut named: BTreeMap<String, (String, Vec<&str>)> = BTreeMap::new();
    for (id, name) in members {
        let name = name.trim();
        if id.is_empty() || name.is_empty() || name.contains(['\r', '\n']) {
            continue;
        }
        let (_, ids) = named
            .entry(folded(name).collect())
            .or_insert_with(|| (name.to_owned(), Vec::new()));
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let mut names: Vec<&(String, Vec<&str>)> = named.values().collect();
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.chars().count()));
    let code = code_ranges(text);
    let mut found: Vec<String> = Vec::new();
    for (at, _) in text.match_indices('@') {
        if code.iter().any(|range| range.contains(&at))
            || text[..at].chars().next_back().is_some_and(word_char_before)
        {
            continue;
        }
        let rest = &text[at + 1..];
        let Some((ids, end)) = names
            .iter()
            .find_map(|(name, ids)| prefix_len(rest, name).map(|end| (ids, end)))
        else {
            continue;
        };
        if rest[end..].chars().next().is_some_and(word_char_after) {
            continue;
        }
        if let [id] = ids.as_slice()
            && !found.iter().any(|known| known == id)
        {
            found.push((*id).to_owned());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: [(&str, &str); 4] = [
        ("bot_story", "Story"),
        ("bot_tiro", "Tiro"),
        ("bot_digest", "Research digest"),
        ("user_felix", "Felix"),
    ];

    #[test]
    fn names_members_as_whole_words_in_any_case() {
        assert_eq!(
            named_mentions(
                "@Story: Das angehängte PDF ist die neue Fassung. Danke @felix!",
                ROOM
            ),
            ["bot_story", "user_felix"]
        );
        assert_eq!(
            named_mentions("(@tiro) and @Story, then @Story again", ROOM),
            ["bot_tiro", "bot_story"]
        );
    }

    #[test]
    fn the_longest_name_wins_and_parts_of_words_name_no_one() {
        let room = [("bot_r", "Research"), ("bot_digest", "Research digest")];
        assert_eq!(
            named_mentions("@Research digest, look", room),
            ["bot_digest"]
        );
        assert_eq!(named_mentions("@Research, look", room), ["bot_r"]);
        assert!(named_mentions("@Storyteller mail@Story @Story_x @Story-x", ROOM).is_empty());
    }

    #[test]
    fn code_and_shared_names_name_no_one() {
        assert!(named_mentions("Write `@Story` or\n```\n@Tiro\n```\n", ROOM).is_empty());
        let twins = [("bot_a", "Scout"), ("bot_b", "scout")];
        assert!(named_mentions("@Scout go", twins).is_empty());
    }

    #[test]
    fn text_without_an_at_sign_or_names_is_left_alone() {
        assert!(named_mentions("Story, please file it", ROOM).is_empty());
        assert!(named_mentions("@Story", [("bot_story", " "), ("", "Story")]).is_empty());
    }

    #[test]
    fn names_that_change_length_when_folded_still_match() {
        let room = [("bot_i", "İlse")];
        assert_eq!(named_mentions("Hi @İLSE.", room), ["bot_i"]);
    }
}
