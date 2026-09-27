//! Check deterministic source generation and the behavior of generated matchers.

use matchgen::{Input, TreeMatcher};
use std::fs;
use std::process::Command;

const ENTRIES: &[(&[u8], &str)] = &[
    (b"a", "1"),
    (b"ab", "2"),
    (b"abc", "3"),
    (b"abd", "4"),
    (b"ac", "5"),
    (b"b", "6"),
    (b"ba", "7"),
    (b"z", "8"),
];

fn render(input: Input, name: &str, entries: &[(&[u8], &str)]) -> String {
    let mut matcher = TreeMatcher::new(format!("fn {name}"), "u64");
    matcher.input_type(input);
    for &(key, value) in entries {
        matcher.add(key, value);
    }
    let mut output = Vec::new();
    matcher.render(&mut output).unwrap();
    String::from_utf8(output).unwrap()
}

fn assert_reproducible(input: Input) {
    let expected = render(input, "matcher", ENTRIES);
    let mut entries = ENTRIES.to_vec();
    for round in 0..4 {
        entries.reverse();
        for rotation in 0..entries.len() {
            entries.rotate_left(1);
            assert_eq!(
                render(input, "matcher", &entries),
                expected,
                "{input:?}, round {round}, rotation {rotation}"
            );
        }
    }
}

#[test]
fn iterator_output_is_reproducible_across_maps_and_insertion_orders() {
    assert_reproducible(Input::Iterator);
}

#[test]
fn slice_output_is_reproducible_across_maps_and_insertion_orders() {
    assert_reproducible(Input::Slice);
}

#[test]
fn generated_matchers_preserve_longest_match_and_input_position() {
    let directory = temp_dir::TempDir::new().unwrap();
    let source = directory.path().join("matcher.rs");
    let binary = directory.path().join(if cfg!(windows) {
        "matcher.exe"
    } else {
        "matcher"
    });
    let main = r#"
fn main() {
    let cases: &[(&[u8], Option<u64>, &[u8])] = &[
        (b"", None, b""),
        (b"q", None, b"q"),
        (b"a", Some(1), b""),
        (b"ax", Some(1), b"x"),
        (b"ab", Some(2), b""),
        (b"abx", Some(2), b"x"),
        (b"abcd!", Some(3), b"d!"),
        (b"abdz", Some(4), b"z"),
        (b"ac!", Some(5), b"!"),
        (b"bx", Some(6), b"x"),
        (b"bark", Some(7), b"rk"),
        (b"zz", Some(8), b"z"),
    ];
    for &(input, expected, remainder) in cases {
        assert_eq!(slice_matcher(input), (expected, remainder));
        let mut iter = input.iter();
        assert_eq!(iterator_matcher(&mut iter), expected);
        assert_eq!(iter.as_slice(), remainder);
    }
}
"#;
    fs::write(
        &source,
        format!(
            "{}\n{}\n{main}",
            render(Input::Slice, "slice_matcher", ENTRIES),
            render(Input::Iterator, "iterator_matcher", ENTRIES)
        ),
    )
    .unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compilation = Command::new(rustc)
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    let execution = Command::new(&binary).output().unwrap();
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
}
