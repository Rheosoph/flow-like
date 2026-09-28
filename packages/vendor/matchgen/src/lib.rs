//! [`TreeMatcher`] can be used from a [build script] to generate a function
//! that accepts bytes as an input and returns a mapped value if it finds a
//! given byte sequence at the start of the input.
//!
//! See [`TreeMatcher`] for example usage.
//!
//! # Minimum supported Rust version
//!
//! Currently the minimum supported Rust version (MSRV) is **1.60**.
//!
//! [build script]: https://doc.rust-lang.org/cargo/reference/build-scripts.html

// Lint configuration in Cargo.toml isn’t supported by cargo-geiger.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Build a function with nested match statements to quickly map byte sequences
/// to values.
///
/// See the variants of [`Input`] for the details about what the generated
/// function will accept and what it returns.
///
/// # Example build script
///
/// ```rust
/// use matchgen::TreeMatcher;
/// use std::error::Error;
///
/// fn main() -> Result<(), Box<dyn Error>> {
///     # let tmp_dir = temp_dir::TempDir::new().unwrap();
///     # std::env::set_var("OUT_DIR", tmp_dir.path());
///     TreeMatcher::new("pub fn fancy_matcher", "&'static [u8]")
///         .doc("My fancy matcher.")
///         .add(b"one", r#"b"1""#)
///         .add(b"two", r#"b"2""#)
///         .add(b"three", r#"b"3""#)
///         .write_to_out_dir("matcher.rs")?;
///
///     Ok(())
/// }
/// ```
///
/// To use the matcher:
///
/// ```rust,ignore
/// include!(concat!(env!("OUT_DIR"), "/matcher.rs"));
///
/// fn main() {
///     assert_eq!(
///         fancy_matcher(b"one two three"),
///         (Some(b"1"), b" two three".as_slice()),
///     );
/// }
/// ```
#[derive(Clone, Debug)]
pub struct TreeMatcher {
    /// The first part of the function definition to generate, e.g.
    /// `"pub fn matcher"`.
    pub fn_name: String,

    /// The return type (will be wrapped in [`Option`]), e.g. `"&'static str"`.
    pub return_type: String,

    /// The type of input to accept. Defaults to [`Input::Slice`].
    pub input_type: Input,

    /// Whether to prevent Clippy from evaluating the generated code. Defaults
    /// to `false`.
    ///
    /// See [`Self::disable_clippy()`].
    pub disable_clippy: bool,

    /// Whether to mark the function with [`#[must_use]`][must_use]. Defaults to
    /// `true`.
    ///
    /// [must_use]: https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-must_use-attribute
    pub must_use: bool,

    /// Doc attribute, e.g. `#[doc = "Documenation"]`, to add to the function.
    ///
    /// Should not have a trailing newline.
    pub doc: Option<String>,

    /// The root of the matcher node tree.
    pub root: TreeNode,
}

impl TreeMatcher {
    /// Create a new matcher (for use in a build script).
    ///
    /// This will generate a matcher with the the specified function name and
    /// return type. You can add matches to it with [`Self::add()`] and/or
    /// [`Self::extend()`], then turn it into code with [`Self::render()`].
    ///
    /// See the [struct documentation][TreeMatcher] for a complete example.
    pub fn new<N, R>(fn_name: N, return_type: R) -> Self
    where
        N: fmt::Display,
        R: fmt::Display,
    {
        Self {
            fn_name: fn_name.to_string(),
            return_type: return_type.to_string(),
            input_type: Input::Slice,
            disable_clippy: false,
            must_use: true,
            doc: None,
            root: TreeNode::default(),
        }
    }

    /// Add a match.
    ///
    /// ```rust
    /// let mut matcher = matchgen::TreeMatcher::new("fn matcher", "u64");
    /// matcher.add(b"a", "1");
    /// ```
    pub fn add<'a, K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: IntoIterator<Item = &'a u8>,
        V: Into<String>,
    {
        self.root.add(key, value);
        self
    }

    /// Set whether or not to prevent [Clippy] from evaluating the generated
    /// code.
    ///
    /// If set to `true`, this will use conditional compilation to prevent
    /// [Clippy] from evaluating the generated code, and will provide a stub
    /// function to ensure that the Clippy pass builds.
    ///
    /// This may be useful if the produced matcher is particularly big. I don’t
    /// recommend setting this to `true` unless you notice a delay when running
    /// `cargo clippy`.
    ///
    /// [Clippy]: https://doc.rust-lang.org/clippy/
    pub fn disable_clippy(&mut self, disable: bool) -> &mut Self {
        self.disable_clippy = disable;
        self
    }

    /// Set whether or not to mark the generated function with [`#[must_use]`].
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// matchgen::TreeMatcher::new("fn match_bytes", "u64")
    ///     .must_use(false)
    ///     .add("a".as_bytes(), "1")
    ///     .render(&mut out)
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     r#"#[allow(clippy::too_many_lines, clippy::single_match_else)]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     match slice.first() {
    ///         Some(97) => (Some(1), &slice[1..]),
    ///         _ => (None, slice),
    ///     }
    /// }
    /// "#,
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    pub fn must_use(&mut self, must_use: bool) -> &mut Self {
        self.must_use = must_use;
        self
    }

    /// Set what kind of input the matcher should accept.
    ///
    /// This can be either:
    ///
    ///   * [`Input::Slice`] to accept `&[u8]`
    ///   * [`Input::Iterator`] to accept `core::iter::Iterator<Item = &'a u8>`
    pub fn input_type(&mut self, input_type: Input) -> &mut Self {
        self.input_type = input_type;
        self
    }

    /// Don’t include documentation for the matcher.
    ///
    /// This is the default behavior.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// matchgen::TreeMatcher::new("fn match_bytes", "u64")
    ///     .remove_doc()
    ///     .add("a".as_bytes(), "1")
    ///     .render(&mut out)
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     r#"#[allow(clippy::too_many_lines, clippy::single_match_else)]
    /// #[must_use]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     match slice.first() {
    ///         Some(97) => (Some(1), &slice[1..]),
    ///         _ => (None, slice),
    ///     }
    /// }
    /// "#,
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    pub fn remove_doc(&mut self) -> &mut Self {
        self.doc = None;
        self
    }

    /// Set documentation for the matcher to a string.
    ///
    /// This is normally what you want. Just pass a string with the
    /// documentation for your matcher, and it will produce an [attribute][]
    /// like `#[doc = "My function documentation..."]`.
    ///
    /// The `doc` argument should produce a Rust string literal when rendered
    /// with [`fmt::Debug`]. A normal [`String`] or [`str`] will work.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// matchgen::TreeMatcher::new("fn match_bytes", "u64")
    ///     .doc(
    ///         "Match the first bytes of a slice.
    ///
    ///         Matches only the string `\"a\"` and returns `1`."
    ///     )
    ///     .add("a".as_bytes(), "1")
    ///     .render(&mut out)
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     r#"#[allow(clippy::too_many_lines, clippy::single_match_else)]
    /// #[doc = "Match the first bytes of a slice.\n\n        Matches only the string `\"a\"` and returns `1`."]
    /// #[must_use]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     match slice.first() {
    ///         Some(97) => (Some(1), &slice[1..]),
    ///         _ => (None, slice),
    ///     }
    /// }
    /// "#,
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    ///
    /// [attribute]: https://doc.rust-lang.org/rustdoc/write-documentation/the-doc-attribute.html
    pub fn doc<S: fmt::Debug>(&mut self, doc: S) -> &mut Self {
        self.doc = Some(format!("#[doc = {doc:?}]"));
        self
    }

    /// Set documentation for the matcher to a Rust expression.
    ///
    /// Generally you want [`Self::doc()`], not this. This can produce an
    /// [attribute][] like `#[doc = include_str!("my_func.md")]`.
    ///
    /// The `doc` argument should produce a Rust expression when rendered with
    /// [`fmt::Display`]. A normal [`String`] or [`str`] will work.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// matchgen::TreeMatcher::new("fn match_bytes", "u64")
    ///     .doc_raw(r#"include_str!("match_bytes.md")"#)
    ///     .add("a".as_bytes(), "1")
    ///     .render(&mut out)
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     r#"#[allow(clippy::too_many_lines, clippy::single_match_else)]
    /// #[doc = include_str!("match_bytes.md")]
    /// #[must_use]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     match slice.first() {
    ///         Some(97) => (Some(1), &slice[1..]),
    ///         _ => (None, slice),
    ///     }
    /// }
    /// "#,
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    ///
    /// [attribute]: https://doc.rust-lang.org/rustdoc/write-documentation/the-doc-attribute.html
    pub fn doc_raw<S: fmt::Display>(&mut self, doc: S) -> &mut Self {
        self.doc = Some(format!("#[doc = {doc}]"));
        self
    }

    /// Set documentation for the matcher to an option.
    ///
    /// Generally you want [`Self::doc()`], not this. This can produce an
    /// [attribute][] like `#[doc(hidden)]`.
    ///
    /// The `doc` argument should produce the options when rendered with
    /// [`fmt::Display`]. A normal [`String`] or [`str`] will work.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// matchgen::TreeMatcher::new("fn match_bytes", "u64")
    ///     .doc_option("hidden")
    ///     .render(&mut out)
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     r#"#[allow(clippy::too_many_lines, clippy::single_match_else)]
    /// #[doc(hidden)]
    /// #[must_use]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     (None, slice)
    /// }
    /// "#,
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    ///
    /// [attribute]: https://doc.rust-lang.org/rustdoc/write-documentation/the-doc-attribute.html
    pub fn doc_option<S: fmt::Display>(&mut self, doc: S) -> &mut Self {
        self.doc = Some(format!("#[doc({doc})]"));
        self
    }

    /// Write the matcher as a Rust source file in `$OUT_DIR`.
    ///
    /// This is what you want if you’re using this in `build.rs` as intended.
    /// See the [`TreeMatcher`] for a full example.
    ///
    /// This will overwrite the file if it already exists, or create a new file
    /// if it does not.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::env::var;
    /// use std::path::Path;
    /// use std::fs;
    ///
    /// # let tmp_dir = temp_dir::TempDir::new().unwrap();
    /// # std::env::set_var("OUT_DIR", tmp_dir.path());
    /// matchgen::TreeMatcher::new("fn match_bytes", "u64")
    ///     .add("a".as_bytes(), "1")
    ///     .write_to_out_dir("matcher.rs")
    ///     .unwrap();
    ///
    /// let path = Path::new(&var("OUT_DIR").unwrap()).join("matcher.rs");
    /// assert!(fs::read_to_string(path).unwrap().contains("fn match_bytes"));
    /// ```
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing the file, or
    /// if `$OUT_DIR` isn’t set to a UTF-8 string.
    pub fn write_to_out_dir<P: AsRef<Path>>(
        &self,
        sub_path: P,
    ) -> io::Result<()> {
        let out_dir = &env::var("OUT_DIR")
            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
        self.write_to_path(Path::new(out_dir).join(sub_path))
    }

    /// Write the matcher as a Rust source file at `path`.
    ///
    /// This will overwrite the file if it already exists, or create a new file
    /// if it does not.
    ///
    /// # Example
    ///
    /// ```rust
    /// # let tmp_dir = temp_dir::TempDir::new().unwrap();
    /// # let path = tmp_dir.child("test.rs");
    /// matchgen::TreeMatcher::new("fn match_bytes", "u64")
    ///     .add("a".as_bytes(), "1")
    ///     .write_to_path(&path)
    ///     .unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing to `path`.
    pub fn write_to_path<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let mut out = io::BufWriter::new(fs::File::create(path)?);
        self.render(&mut out)?;
        out.flush()
    }

    /// Render the matcher into Rust code.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// let mut matcher = matchgen::TreeMatcher::new("fn match_bytes", "u64");
    /// matcher
    ///     .disable_clippy(true)
    ///     .add("a".as_bytes(), "1")
    ///     .render(&mut out)
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     r#"#[cfg(not(clippy))]
    /// #[must_use]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     match slice.first() {
    ///         Some(97) => (Some(1), &slice[1..]),
    ///         _ => (None, slice),
    ///     }
    /// }
    ///
    /// #[cfg(clippy)]
    /// #[must_use]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     (None, slice)
    /// }
    /// "#,
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing to `writer`.
    pub fn render<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.disable_clippy {
            writeln!(writer, "#[cfg(not(clippy))]")?;
        }

        self.render_func(writer)?;

        if self.disable_clippy {
            writeln!(writer)?;
            writeln!(writer, "#[cfg(clippy)]")?;
            self.render_stub(writer)?;
        }

        Ok(())
    }

    /// Render the function that does the matching.
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing to `writer`.
    fn render_func<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        if !self.disable_clippy {
            writeln!(
                writer,
                "#[allow(clippy::too_many_lines, clippy::single_match_else)]"
            )?;
        }

        self.render_attributes(writer)?;

        match self.input_type {
            Input::Slice => {
                self.root
                    .render_slice(writer, &self.fn_name, &self.return_type)
            }
            Input::Iterator => {
                self.root
                    .render_iter(writer, &self.fn_name, &self.return_type)
            }
        }
    }

    /// Render a stub that does nothing.
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing to `writer`.
    fn render_stub<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        self.render_attributes(writer)?;

        match self.input_type {
            Input::Slice => TreeNode::default().render_slice(
                writer,
                &self.fn_name,
                &self.return_type,
            ),
            Input::Iterator => TreeNode::default().render_iter(
                writer,
                &self.fn_name,
                &self.return_type,
            ),
        }
    }

    /// Render attributes for the function or stub.
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing to `writer`.
    fn render_attributes<W: io::Write>(
        &self,
        writer: &mut W,
    ) -> io::Result<()> {
        if let Some(doc) = &self.doc {
            writeln!(writer, "{doc}")?;
        }

        if self.must_use {
            writeln!(writer, "#[must_use]")?;
        }

        Ok(())
    }
}

impl<'a, K, V> Extend<(K, V)> for TreeMatcher
where
    K: IntoIterator<Item = &'a u8>,
    V: Into<String>,
{
    /// Add matches from iterator or collection.
    ///
    /// See trait documentation in [`core::iter::Extend`].
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// let mut matcher = matchgen::TreeMatcher::new("fn match_bytes", "u64");
    /// matcher.extend([("a".as_bytes(), "1")]);
    /// matcher.render(&mut out).unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     r#"#[allow(clippy::too_many_lines, clippy::single_match_else)]
    /// #[must_use]
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     match slice.first() {
    ///         Some(97) => (Some(1), &slice[1..]),
    ///         _ => (None, slice),
    ///     }
    /// }
    /// "#,
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        iter.into_iter().for_each(|(key, value)| {
            self.add(key, value);
        });
    }
}

/// What kind of input the matcher should accept: a slice or an iterator.
#[derive(Clone, Copy, Debug)]
pub enum Input {
    /// Accept a slice of bytes.
    ///
    /// The generated function will accept a slice of bytes (`&[u8]`) and will
    /// return a tuple containing the match, if any, and the remainder of the
    /// slice, i.e. `(Option<{return_type}>, &[u8])`.
    ///
    /// For example, suppose you generate a [matcher for all HTML
    /// entities][htmlize] called `entity_matcher()`:
    ///
    /// ```rust,ignore
    /// assert!(entity_matcher(b"&times;XY") == (Some("×"), b"XY".as_slice()));
    /// ```
    ///
    ///   * The prefixes it checks do not all have to be the same length.
    ///   * If more than one prefix matches, it will return the longest one.
    ///   * If nothing matches, it will return `(None, &input)`.
    ///
    /// Since the matchers only check the start of the input, you will want to
    /// use [`iter().position()`] or the [memchr crate][memchr] to find the
    /// start of a potential match.
    ///
    /// [`iter().position()`]: https://doc.rust-lang.org/std/iter/trait.Iterator.html#method.position
    /// [memchr]: http://docs.rs/memchr
    /// [htmlize]: https://crates.io/crates/htmlize
    Slice,

    /// Accept an iterator over bytes.
    ///
    /// The generated function will accept an iterator over bytes
    /// (`core::iter::Iterator<Item = &'a u8>`) and will return a match if it
    /// finds a given byte sequence at the start of the iterator. It will update
    /// the iterator to move past the match (and no farther).
    ///
    /// For example, suppose you generate a [matcher for all HTML
    /// entities][htmlize] called `entity_matcher()`:
    ///
    /// ```rust,ignore
    /// let mut iter = b"&times;XY".iter();
    /// assert!(entity_matcher(&mut iter) == Some("×"));
    /// assert!(iter.next() == Some(b'X'));
    /// ```
    ///
    ///   * **The prefixes it checks do not all have to be the same length.**
    ///     The matcher will clone the iterator if it needs to look ahead, so
    ///     when the matcher returns the iterator will only have consumed what
    ///     was matched.
    ///   * **If more than one prefix matches, it will return the longest one.**
    ///   * **If nothing matches, the iterator will not be advanced.** You may
    ///     want to call `iterator.next()` if the matcher returns `None`.
    ///   * **It only checks the start of the iterator.** Often you will want to
    ///     use [`position()`] or the [memchr crate][memchr] to find the start
    ///     of a potential match.
    ///
    /// [`position()`]: https://doc.rust-lang.org/std/iter/trait.Iterator.html#method.position
    /// [memchr]: http://docs.rs/memchr
    /// [htmlize]: https://crates.io/crates/htmlize
    Iterator,
}

impl Default for Input {
    fn default() -> Self {
        Self::Slice
    }
}

/// A node in a tree matcher’s simple finite-state automaton.
///
/// You probably want to use [`TreeMatcher`] instead.
#[derive(Clone, Debug, Default)]
pub struct TreeNode {
    /// If the matcher gets to this node and `leaf` is `Some(_)`, then we found
    /// a (potential) match.
    ///
    /// If `branch` is not empty, then we still need to check further on to see
    /// if there’s a longer match.
    pub leaf: Option<String>,

    /// The list of characters that could be matched next, and the nodes they
    /// represent.
    ///
    /// If none of these characters match, then return `leaf` as the match
    /// (it might be None, indicating that nothing matches).
    pub branch: HashMap<u8, Self>,
}

impl TreeNode {
    /// Add a match rooted in this node.
    ///
    /// ```rust
    /// let mut node = matchgen::TreeNode::default();
    /// node.add(b"a", "1");
    /// ```
    pub fn add<'a, K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: IntoIterator<Item = &'a u8>,
        V: Into<String>,
    {
        /// Without this internal function `self` is moved into `fold()` and
        /// thus cannot be returned.
        #[inline]
        fn internal<'a, K: Iterator<Item = &'a u8>>(
            node: &mut TreeNode,
            key: K,
            value: String,
        ) {
            let node =
                key.fold(node, |node, &c| node.branch.entry(c).or_default());
            node.leaf = Some(value);
        }
        internal(self, key.into_iter(), value.into());
        self
    }

    /// Render the matcher into Rust code that works on an iterator.
    ///
    /// The parameters are:
    ///
    ///   1. An instance of [`std::io::Write`], like [`std::io::stdout()`], a
    ///      file, or a [`Vec`].
    ///   2. The first part of the function definition to generate, e.g.
    ///      `"pub fn matcher"`.
    ///   3. The return type (will be wrapped in [`Option`]), e.g.
    ///      `"&'static str"`.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// matchgen::TreeNode::from_iter([("a".as_bytes(), "1")])
    ///     .render_iter(&mut out, "fn match_bytes", "u64")
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     "\
    /// fn match_bytes<'a, I>(iter: &mut I) -> Option<u64>
    /// where
    ///     I: core::iter::Iterator<Item = &'a u8> + core::clone::Clone,
    /// {
    ///     let fallback_iter = iter.clone();
    ///     match iter.next() {
    ///         Some(97) => Some(1),
    ///         _ => {
    ///             *iter = fallback_iter;
    ///             None
    ///         }
    ///     }
    /// }
    /// ",
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing to `writer`.
    #[allow(clippy::items_after_statements)]
    pub fn render_iter<W, N, R>(
        &self,
        writer: &mut W,
        fn_name: N,
        return_type: R,
    ) -> io::Result<()>
    where
        W: io::Write,
        N: fmt::Display,
        R: fmt::Display,
    {
        let indent = "    "; // Our formatting prevents embedding this.

        if self.branch.is_empty() {
            // Special handling for when no matches were added.
            write!(
                writer,
                "{fn_name}<'a, I>(_iter: &mut I) -> Option<{return_type}>\n\
                where\n\
                {indent}I: core::iter::Iterator<Item = &'a u8> + core::clone::Clone,\n\
                {{\n\
                {indent}")?;
            render_child(self, writer, 0, None)?;
            writeln!(writer, "\n}}")?;
        } else {
            write!(
                writer,
                "{fn_name}<'a, I>(iter: &mut I) -> Option<{return_type}>\n\
                where\n\
                {indent}I: core::iter::Iterator<Item = &'a u8> + core::clone::Clone,\n"
            )?;
            render_child(self, writer, 0, None)?;
            writeln!(writer)?;
        }

        // FIXME: this is recursive, so for long patterns it could blow out the
        // stack. Transform this to an iterative algorithm.

        /// Render a node: handle nodes that are a leaf, are a branch, or both.
        #[inline]
        fn render_child<W: io::Write>(
            node: &TreeNode,
            writer: &mut W,
            level: usize,
            fallback: Option<&String>,
        ) -> io::Result<()> {
            if node.branch.is_empty() {
                // Terminal. node.leaf should be Some(_), but might not be.
                write!(
                    writer,
                    "{}",
                    leaf_to_str(node.leaf.as_ref().or(fallback))
                )?;
            } else if node.leaf.is_none() && level > 0 {
                // No patterns end here: branch only. (The level check creates
                // a default root pattern of `[] => None` so that we rewind the
                // iter when nothing matches.)
                render_match(node, writer, level, fallback)?;
            } else {
                // A pattern ends here.
                let indent = "    ".repeat(level);
                write!(
                    writer,
                    "{{\n\
                    {indent}    let fallback_iter = iter.clone();\n\
                    {indent}    "
                )?;
                render_match(
                    node,
                    writer,
                    level.checked_add(1).unwrap(),
                    node.leaf.as_ref(),
                )?;
                write!(writer, "\n{indent}}}")?;
            }

            Ok(())
        }

        /// Render a match statement to find the next node based on the next
        /// character, i.e. renders `node.branch`.
        #[inline]
        fn render_match<W: io::Write>(
            node: &TreeNode,
            writer: &mut W,
            level: usize,
            fallback: Option<&String>,
        ) -> io::Result<()> {
            let indent = "    ".repeat(level);
            writeln!(writer, "match iter.next() {{")?;
            // Stable branch order keeps generated code identical across builds.
            let mut branches: Vec<_> = node.branch.iter().collect();
            branches.sort_unstable_by_key(|(chunk, _)| **chunk);
            for (chunk, child) in branches {
                write!(writer, "{indent}    Some({chunk:?}) => ")?;
                render_child(
                    child,
                    writer,
                    level.checked_add(1).unwrap(),
                    fallback,
                )?;
                if child.branch.is_empty() {
                    // render_child() wrote a value, not a match block.
                    writeln!(writer, ",")?;
                } else {
                    // render_child() wrote a match block.
                    writeln!(writer)?;
                }
            }

            let fallback = leaf_to_str(fallback);
            write!(
                writer,
                "{indent}    _ => {{\n\
                {indent}        *iter = fallback_iter;\n\
                {indent}        {fallback}\n\
                {indent}    }}\n\
                {indent}}}"
            )?;

            Ok(())
        }

        /// Format `Option<"Rust code">` as Rust code.
        #[inline]
        fn leaf_to_str(leaf: Option<&String>) -> String {
            leaf.map(|leaf| format!("Some({leaf})"))
                .unwrap_or_else(|| "None".to_owned())
        }

        Ok(())
    }

    /// Render the matcher into Rust code that works on a slice.
    ///
    /// The parameters are:
    ///
    ///   1. An instance of [`std::io::Write`], like [`std::io::stdout()`], a
    ///      file, or a [`Vec`].
    ///   2. The first part of the function definition to generate, e.g.
    ///      `"pub fn matcher"`.
    ///   3. The return type (will be wrapped in [`Option`] and a tuple), e.g.
    ///      `"&'static str"`.
    ///
    /// # Example
    ///
    /// ```rust
    /// let mut out = Vec::new();
    /// matchgen::TreeNode::from_iter([("a".as_bytes(), "1")])
    ///     .render_slice(&mut out, "fn match_bytes", "u64")
    ///     .unwrap();
    ///
    /// use bstr::ByteVec;
    /// pretty_assertions::assert_str_eq!(
    ///     "\
    /// fn match_bytes(slice: &[u8]) -> (Option<u64>, &[u8]) {
    ///     match slice.first() {
    ///         Some(97) => (Some(1), &slice[1..]),
    ///         _ => (None, slice),
    ///     }
    /// }
    /// ",
    ///     out.into_string().unwrap(),
    /// );
    /// ```
    ///
    /// # Errors
    ///
    /// This can return [`io::Error`] if there is a problem writing to `writer`.
    #[allow(clippy::items_after_statements)]
    pub fn render_slice<W, N, R>(
        &self,
        writer: &mut W,
        fn_name: N,
        return_type: R,
    ) -> io::Result<()>
    where
        W: io::Write,
        N: fmt::Display,
        R: fmt::Display,
    {
        let indent = "    "; // Our formatting prevents embedding this.

        write!(
            writer,
            "{fn_name}(slice: &[u8]) -> (Option<{return_type}>, &[u8]) {{\n\
            {indent}",
        )?;
        render_child(self, writer, 0, None)?;
        writeln!(writer, "\n}}")?;

        // FIXME: this is recursive, so for long patterns it could blow out the
        // stack. Transform this to an iterative algorithm.

        /// Render a node: handle nodes that are a leaf, are a branch, or both.
        #[inline]
        fn render_child<W: io::Write>(
            node: &TreeNode,
            writer: &mut W,
            index: usize,
            fallback: Option<(&String, usize)>,
        ) -> io::Result<()> {
            if node.branch.is_empty() {
                // Terminal. node.leaf should be Some(_), but might not be.
                if let Some(leaf) = &node.leaf {
                    write!(writer, "(Some({leaf}), &slice[{index}..])")
                } else if let Some((value, fallback)) = fallback {
                    write!(writer, "(Some({value}), &slice[{fallback}..])")
                } else {
                    write!(writer, "(None, slice)")
                }
            } else {
                if index == 0 {
                    // For Clippy.
                    writeln!(writer, "match slice.first() {{")?;
                } else {
                    writeln!(writer, "match slice.get({index}) {{")?;
                }

                let fallback =
                    node.leaf.as_ref().map(|leaf| (leaf, index)).or(fallback);
                let next_index = index.checked_add(1).unwrap();
                let indent = "    ".repeat(next_index);

                let mut branches: Vec<_> = node.branch.iter().collect();
                branches.sort_unstable_by_key(|(chunk, _)| **chunk);
                for (chunk, child) in branches {
                    write!(writer, "{indent}    Some({chunk:?}) => ")?;
                    render_child(child, writer, next_index, fallback)?;
                    if child.branch.is_empty() {
                        // render_child() wrote a value, not a match block.
                        writeln!(writer, ",")?;
                    } else {
                        // render_child() wrote a match block.
                        writeln!(writer)?;
                    }
                }

                let default = if let Some((value, index)) = fallback {
                    format!("(Some({value}), &slice[{index}..])")
                } else {
                    "(None, slice)".to_owned()
                };

                write!(
                    writer,
                    "{indent}    _ => {default},\n\
                    {indent}}}"
                )
            }
        }

        Ok(())
    }
}

impl<'a, K, V> FromIterator<(K, V)> for TreeNode
where
    K: IntoIterator<Item = &'a u8>,
    V: Into<String>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut root = Self::default();
        root.extend(iter);
        root
    }
}

impl<'a, K, V> Extend<(K, V)> for TreeNode
where
    K: IntoIterator<Item = &'a u8>,
    V: Into<String>,
{
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, iter: I) {
        iter.into_iter().for_each(|(key, value)| {
            self.add(key, value);
        });
    }
}
