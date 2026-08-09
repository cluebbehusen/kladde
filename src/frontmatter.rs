//! Reading and editing a note's frontmatter.
//!
//! The format is the flat properties subset markdown note editors read:
//! a block of `key: value` lines between `---` fences at the very start
//! of the file, holding text values and lists. Types beyond that
//! (numbers, checkboxes, dates) are spellings of text values, because
//! editors that type properties keep the type outside the note.
//! Everything the subset does not cover — nested values, block scalars,
//! comments — is opaque: preserved byte for byte, never edited through,
//! and reported as an error when named directly. An edit rewrites only
//! the named property, in one canonical form; every other byte of the
//! note survives untouched.
//!
//! Where the convention is silent, the boundary was measured
//! empirically: a UTF-8 byte order mark before the opening
//! fence is tolerated, and fences may end in CRLF; a leading blank line,
//! trailing spaces on a fence, an unclosed fence, and YAML's `...` closer
//! are not frontmatter. An empty block between two fences is recognized.
//! Edits inside a CRLF block stay CRLF; new blocks are written with LF.

/// Property name for the created stamp when `stamp-created-key` is
/// unset.
pub const DEFAULT_CREATED_KEY: &str = "created";

/// Property name for the updated stamp when `stamp-updated-key` is
/// unset.
pub const DEFAULT_UPDATED_KEY: &str = "updated";

/// A property's value: text, or a list of texts.
///
/// Values are logical: quoting is a spelling concern of the file, so a
/// value quoted in the note comes back without its quotes, and a value
/// set through this module is quoted only when the format requires it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A single-line text value.
    Scalar(String),
    /// A list of single-line text values.
    List(Vec<String>),
}

/// Failure while reading or editing a property.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid property key \"{}\": {reason}", key.escape_debug())]
    InvalidKey { key: String, reason: &'static str },
    #[error("property values are single lines, got \"{}\"", value.escape_debug())]
    MultilineValue { value: String },
    #[error("the frontmatter is a single value, not properties")]
    ForeignBlock,
    #[error("property values cannot hold unprintable characters, got \"{}\"", value.escape_debug())]
    UnprintableValue { value: String },
    #[error("multiple properties named \"{key}\"")]
    DuplicateKey { key: String },
    #[error("property \"{key}\" is not a text or list value")]
    Opaque { key: String },
    #[error("property \"{key}\" is not a list")]
    NotAList { key: String },
}

/// A note split at the frontmatter boundary. `head` is the byte order
/// mark preceding an opening fence, if any; `body` is the byte-exact
/// remainder after the closing fence, or the whole note when there is no
/// block.
struct Document {
    head: String,
    block: Option<Block>,
    body: String,
}

/// A frontmatter block: its fence lines verbatim, and its interior.
struct Block {
    opening: String,
    items: Vec<Item>,
    closing: String,
}

/// One interior unit of a block: a property with its lines, or a single
/// raw line (a comment, a blank, a stray line) preserved verbatim.
enum Item {
    Property(Property),
    Raw(String),
}

/// A property: its key, its logical value, and its original lines,
/// emitted verbatim while the property is untouched. An edited property
/// carries freshly rendered canonical lines instead.
struct Property {
    key: String,
    value: Parsed,
    raw: Vec<String>,
}

/// A parsed value. Unlike [`Value`], parsing can also find a shape the
/// subset does not cover; such a property is preserved but only
/// overwritten or removed whole, never read or edited item-wise.
enum Parsed {
    Scalar(String),
    List(Vec<String>),
    Opaque,
}

/// Reads the value of `key`, or `None` when the note has no frontmatter
/// or no such property.
///
/// # Errors
///
/// Returns an error when `key` is not a valid property key, names
/// duplicated properties, or names a property whose value is outside the
/// subset this module edits.
pub fn get(text: &str, key: &str) -> Result<Option<Value>, Error> {
    validated_key(key)?;
    let document = parse(text);
    let Some(block) = &document.block else {
        return Ok(None);
    };
    match current_value(block, key)? {
        None => Ok(None),
        Some(Parsed::Scalar(value)) => Ok(Some(Value::Scalar(value.clone()))),
        Some(Parsed::List(items)) => Ok(Some(Value::List(items.clone()))),
        Some(Parsed::Opaque) => Err(Error::Opaque {
            key: key.to_owned(),
        }),
    }
}

/// Sets `key` to the text value `value`, replacing any existing value
/// whatever its shape, and creating the frontmatter block if the note has
/// none.
///
/// # Errors
///
/// Returns an error when `key` is not a valid property key or names
/// duplicated properties, or when `value` spans lines.
pub fn set(text: &str, key: &str, value: &str) -> Result<String, Error> {
    validated_key(key)?;
    validated_value(value)?;
    let mut document = parse(text);
    let block = document.block.get_or_insert_with(new_block);
    if foreign_rooted(block) {
        return Err(Error::ForeignBlock);
    }
    current_value(block, key)?;
    let eol = block_eol(block);
    put(block, scalar_property(key, value, eol));
    Ok(render(&document))
}

/// Removes the property `key`. A missing property, or a note without
/// frontmatter, is already the requested state and returns the note
/// unchanged. Removing the last interior line removes the fences too —
/// unless the body would then read as a block itself, in which case an
/// empty block stays behind to fence it off.
///
/// # Errors
///
/// Returns an error when `key` is not a valid property key, names
/// duplicated properties, or when the frontmatter is not property lines.
pub fn unset(text: &str, key: &str) -> Result<String, Error> {
    validated_key(key)?;
    let mut document = parse(text);
    let Some(block) = &mut document.block else {
        return Ok(text.to_owned());
    };
    if foreign_rooted(block) {
        return Err(Error::ForeignBlock);
    }
    if current_value(block, key)?.is_none() {
        return Ok(text.to_owned());
    }
    block
        .items
        .retain(|item| !matches!(item, Item::Property(property) if property.key == key));
    if block.items.is_empty() && !has_block(&document.body) {
        document.block = None;
    }
    Ok(render(&document))
}

/// Adds `item` to the list property `key`, creating the note's block and
/// the property as needed. An item already present is already the
/// requested state and returns the note unchanged.
///
/// # Errors
///
/// Returns an error when `key` is not a valid property key, names
/// duplicated properties, or holds a text or out-of-subset value, or
/// when `item` spans lines.
pub fn add(text: &str, key: &str, item: &str) -> Result<String, Error> {
    validated_key(key)?;
    validated_value(item)?;
    let mut document = parse(text);
    let block = document.block.get_or_insert_with(new_block);
    if foreign_rooted(block) {
        return Err(Error::ForeignBlock);
    }
    let mut items = current_list(block, key)?.unwrap_or_default();
    if items.iter().any(|existing| existing == item) {
        return Ok(text.to_owned());
    }
    items.push(item.to_owned());
    let eol = block_eol(block);
    put(block, list_property(key, items, eol));
    Ok(render(&document))
}

/// Removes `item` from the list property `key`. An absent item, a
/// missing property, or a note without frontmatter is already the
/// requested state and returns the note unchanged; removing the last
/// item leaves an empty list.
///
/// # Errors
///
/// Returns an error when `key` is not a valid property key, names
/// duplicated properties, or holds a text or out-of-subset value, when
/// `item` spans lines, or when the frontmatter is not property lines.
pub fn remove(text: &str, key: &str, item: &str) -> Result<String, Error> {
    validated_key(key)?;
    validated_value(item)?;
    let mut document = parse(text);
    let Some(block) = &mut document.block else {
        return Ok(text.to_owned());
    };
    if foreign_rooted(block) {
        return Err(Error::ForeignBlock);
    }
    let Some(items) = current_list(block, key)? else {
        return Ok(text.to_owned());
    };
    let remaining: Vec<String> = items
        .iter()
        .filter(|existing| *existing != item)
        .cloned()
        .collect();
    if remaining.len() == items.len() {
        return Ok(text.to_owned());
    }
    let eol = block_eol(block);
    put(block, list_property(key, remaining, eol));
    Ok(render(&document))
}

/// Whether the note begins with a well-formed frontmatter block.
#[must_use]
pub fn has_block(text: &str) -> bool {
    parse(text).block.is_some()
}

/// The stamp properties a write records: who created the block, who last
/// wrote the note, and the already-rendered timestamp for both. A stamp
/// can only be built through [`Stamp::new`], which validates its parts,
/// so holding one is proof the stamp renders cleanly.
#[derive(Debug)]
pub struct Stamp<'a> {
    /// Property name recording when kladde created the block.
    created_key: &'a str,
    /// Property name recording when kladde last wrote the note.
    updated_key: &'a str,
    /// The rendered timestamp value.
    timestamp: &'a str,
}

impl<'a> Stamp<'a> {
    /// Builds a stamp whose parts are proven writable: the keys satisfy
    /// the property-key rules and the timestamp is a valid single-line
    /// value, so rendering the stamp can never malform a note.
    ///
    /// # Errors
    ///
    /// Returns an error when either key cannot name a property or the
    /// timestamp cannot be a property value.
    pub fn new(
        created_key: &'a str,
        updated_key: &'a str,
        timestamp: &'a str,
    ) -> Result<Self, Error> {
        validated_key(created_key)?;
        validated_key(updated_key)?;
        validated_value(timestamp)?;
        Ok(Self {
            created_key,
            updated_key,
            timestamp,
        })
    }
}

/// Stamps a note after an edit: the updated stamp is set, and the
/// created stamp too when the edit made a block where `had_block` says
/// there was none — a scalar stamp key the same edit set by hand
/// survives. Stamping is best effort and all or nothing: when the edit
/// removed the block, or either stamp key exists as a list, a duplicate,
/// or an out-of-subset value, the note is returned unchanged rather than
/// mangled — capturing the edit outranks stamp fidelity.
#[must_use]
pub fn stamped(text: &str, had_block: bool, stamp: &Stamp<'_>) -> String {
    let mut document = parse(text);
    if document.block.is_none() && had_block {
        return text.to_owned();
    }
    let block = document.block.get_or_insert_with(new_block);
    if foreign_rooted(block) {
        return text.to_owned();
    }
    match current_value(block, stamp.updated_key) {
        Ok(None | Some(Parsed::Scalar(_))) => {}
        Ok(Some(_)) | Err(_) => return text.to_owned(),
    }
    let eol = block_eol(block);
    if !had_block {
        let created = match current_value(block, stamp.created_key) {
            Ok(None) => true,
            Ok(Some(Parsed::Scalar(_))) => false,
            Ok(Some(_)) | Err(_) => return text.to_owned(),
        };
        if created {
            put(
                block,
                scalar_property(stamp.created_key, stamp.timestamp, eol),
            );
        }
    }
    put(
        block,
        scalar_property(stamp.updated_key, stamp.timestamp, eol),
    );
    render(&document)
}

/// Checks that `key` can name a property: the rejections keep every key
/// this module writes parseable as a key here and wherever the note is
/// read. The config module shares these rules for the stamp key
/// settings, so a configured stamp key is always writable.
pub(crate) fn validated_key(key: &str) -> Result<(), Error> {
    let reason = if key.is_empty() {
        Some("it is empty")
    } else if key.contains(':') {
        Some("it contains a colon")
    } else if key.contains('#') {
        Some("it contains a hash")
    } else if key.contains(['\n', '\r']) {
        Some("it contains a line break")
    } else if key
        .chars()
        .any(|character| character.is_control() || forbidden_character(character))
    {
        Some("it contains an unprintable character")
    } else if key.trim() != key {
        Some("it has leading or trailing whitespace")
    } else if key.starts_with([
        '-', '"', '\'', '&', '*', '!', '%', '@', '`', '?', '|', '>', '[', ']', '{', '}', ',',
    ]) {
        Some("it starts with a character YAML reserves")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(Error::InvalidKey {
            key: key.to_owned(),
            reason,
        }),
        None => Ok(()),
    }
}

/// Checks that a value or list item stays on one line and carries no
/// control characters the quoted form cannot escape; a tab is the one
/// exception, since it survives inside quotes.
fn validated_value(value: &str) -> Result<(), Error> {
    if value.contains(['\n', '\r']) {
        return Err(Error::MultilineValue {
            value: value.to_owned(),
        });
    }
    if holds_forbidden_character(value) {
        return Err(Error::UnprintableValue {
            value: value.to_owned(),
        });
    }
    Ok(())
}

/// Whether a character can never appear in a written property: a
/// control character other than tab — unprintable, and raw escape
/// sequences could drive the reader's terminal — one of the two
/// noncharacters YAML's printable set excludes, or one of the two
/// separators older YAML readers treat as line breaks.
pub(crate) fn forbidden_character(character: char) -> bool {
    (character.is_control() && character != '\t')
        || matches!(character, '\u{fffe}' | '\u{ffff}' | '\u{2028}' | '\u{2029}')
}

/// Whether a value holds a character the subset can never write back:
/// parsing treats such values as opaque and mutation rejects them.
fn holds_forbidden_character(value: &str) -> bool {
    value.chars().any(forbidden_character)
}

/// YAML's separation whitespace is the space and the tab, nothing wider:
/// a no-break space or any other Unicode whitespace is content, so
/// parsing only ever trims these two.
fn yaml_trim(value: &str) -> &str {
    value.trim_matches([' ', '\t'])
}

fn yaml_trim_start(value: &str) -> &str {
    value.trim_start_matches([' ', '\t'])
}

fn yaml_trim_end(value: &str) -> &str {
    value.trim_end_matches([' ', '\t'])
}

/// The bytes to skip before the opening fence. Measured empirically: a
/// UTF-8 byte order mark is tolerated there, a leading blank line is
/// not.
fn head_offset(text: &str) -> usize {
    if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    }
}

/// Whether a line (without its terminator) is a fence. Measured
/// empirically: exactly `---`, no trailing spaces, and never YAML's
/// `...`.
fn is_fence(content: &str) -> bool {
    content == "---"
}

/// A line without its terminator, whether that was LF or CRLF.
fn content(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// The line terminator for lines written into the block, matching the
/// opening fence so a CRLF block never gains mixed endings.
fn block_eol(block: &Block) -> &'static str {
    if block.opening.ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// An empty block for a note gaining its first property.
fn new_block() -> Block {
    Block {
        opening: "---\n".to_owned(),
        items: Vec::new(),
        closing: "---\n".to_owned(),
    }
}

/// Splits a note at the frontmatter boundary. Never fails: a note whose
/// head is not a well-formed block is all body.
fn parse(text: &str) -> Document {
    let head = &text[..head_offset(text)];
    let body_only = || Document {
        head: head.to_owned(),
        block: None,
        body: text[head.len()..].to_owned(),
    };
    let mut lines = text[head.len()..].split_inclusive('\n');
    let Some(opening) = lines.next() else {
        return body_only();
    };
    if !opening.ends_with('\n') || !is_fence(content(opening)) {
        return body_only();
    }
    let mut interior = Vec::new();
    let mut consumed = head.len() + opening.len();
    let mut closing = None;
    for line in lines {
        consumed += line.len();
        if is_fence(content(line)) {
            closing = Some(line.to_owned());
            break;
        }
        interior.push(line.to_owned());
    }
    let Some(closing) = closing else {
        return body_only();
    };
    Document {
        head: head.to_owned(),
        block: Some(Block {
            opening: opening.to_owned(),
            items: classify(&interior),
            closing,
        }),
        body: text[consumed..].to_owned(),
    }
}

/// Groups a block's interior lines into properties and raw lines. A
/// property's extent runs while lines can continue it, except that
/// trailing blank and comment lines belong between properties, not to
/// the one above.
fn classify(lines: &[String]) -> Vec<Item> {
    let mut items = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some((key, rest)) = key_line(content(&lines[index])) else {
            items.push(Item::Raw(lines[index].clone()));
            index += 1;
            continue;
        };
        let listable = yaml_trim(&rest).is_empty();
        let mut end = index + 1;
        while end < lines.len() && continues_property(content(&lines[end]), listable) {
            end += 1;
        }
        let mut peeled = end;
        while peeled > index + 1 && between_properties(content(&lines[peeled - 1])) {
            peeled -= 1;
        }
        // An extent that is nothing but commentary carries no value
        // content: those lines stand alone, whatever their indent, and
        // only a value-bearing extent may own an indented comment.
        if lines[index + 1..peeled]
            .iter()
            .all(|line| commentary(content(line)))
        {
            peeled = index + 1;
        }
        let extent: Vec<&str> = lines[index + 1..peeled]
            .iter()
            .map(|line| content(line))
            .collect();
        items.push(Item::Property(Property {
            key,
            value: parsed_value(&rest, &extent),
            raw: lines[index..peeled].to_vec(),
        }));
        for line in &lines[peeled..end] {
            items.push(Item::Raw(line.clone()));
        }
        index = end;
    }
    items
}

/// Whether the block's interior is rooted in something other than
/// property lines: a flow value — tagged or anchored included — a bare
/// scalar, a block sequence, a stray line. YAML gives a block one root,
/// and the first content line decides it — when that is not a property,
/// writing property lines in would corrupt the value, so edits refuse
/// and stamping skips. Commentary carries no content, so it never
/// decides the root.
fn foreign_rooted(block: &Block) -> bool {
    for item in &block.items {
        match item {
            Item::Property(property) => {
                return content(&property.raw[0]).starts_with([
                    '{', '[', ']', '}', '&', '*', '!', '%', '@', '`', '?', '|', '>', ',',
                ]);
            }
            Item::Raw(line) => {
                if !commentary(content(line)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether a line at the end of an extent separates properties rather
/// than continuing one: blank lines, and comments at column zero — an
/// indented comment stays with the extent above it, where a block scalar
/// may own it as content.
fn between_properties(content: &str) -> bool {
    yaml_trim(content).is_empty() || content.starts_with('#')
}

/// Whether a line is commentary — blank, or a comment at any indent —
/// carrying no value content of its own.
fn commentary(content: &str) -> bool {
    yaml_trim(content).is_empty() || yaml_trim_start(content).starts_with('#')
}

/// Whether a line can continue the property above it: an indented
/// continuation, the blank and comment lines the peel above sorts out,
/// or — only under a key line with no inline value, where a list can
/// start — a list item. Any other column-zero line — a quoted key, a
/// stray word, an item after a scalar — is its own top-level line,
/// whatever it says: absorbing it into the property would let an edit of
/// that property silently delete it.
fn continues_property(content: &str, listable: bool) -> bool {
    content.starts_with([' ', '\t'])
        || between_properties(content)
        || (listable && (content == "-" || content.starts_with("- ")))
}

/// Splits a key line into its key and the rest after the colon, or
/// `None` for a line that does not declare a property. Whitespace before
/// the colon is trimmed off the key, as YAML trims it, and a quoted key
/// is read under its unquoted name — so `status : x`, `"status": y`,
/// and `status: z` all name one property, and collide as the duplicates
/// they are.
fn key_line(content: &str) -> Option<(String, String)> {
    let first = content.chars().next()?;
    if first.is_whitespace() || matches!(first, '#' | '-') {
        return None;
    }
    if let Some(tail) = content.strip_prefix('"') {
        let (key, consumed) = scan_double(tail)?;
        return keyed(key, &tail[consumed..]);
    }
    if let Some(tail) = content.strip_prefix('\'') {
        let (key, consumed) = scan_single(tail)?;
        return keyed(key, &tail[consumed..]);
    }
    let (key, rest) = split_at_colon(content)?;
    let key = yaml_trim_end(key);
    if key.is_empty() {
        return None;
    }
    Some((key.to_owned(), rest.to_owned()))
}

/// Pairs a quoted key with the value after its colon, or `None` when no
/// colon follows — then the quote was a value or a stray line, not a
/// key.
fn keyed(key: String, after: &str) -> Option<(String, String)> {
    if key.is_empty() {
        return None;
    }
    let (gap, rest) = split_at_colon(yaml_trim_start(after))?;
    gap.is_empty().then(|| (key, rest.to_owned()))
}

/// Splits a line at the first colon YAML reads as the key separator:
/// one followed by a space, a tab, or the end of the line.
fn split_at_colon(content: &str) -> Option<(&str, &str)> {
    for (index, _) in content.match_indices(':') {
        let tail = &content[index + 1..];
        match tail.chars().next() {
            None => return Some((&content[..index], "")),
            Some(' ' | '\t') => return Some((&content[..index], &tail[1..])),
            Some(_) => {}
        }
    }
    None
}

/// Classifies a property's value from the rest of its key line and its
/// extent.
fn parsed_value(rest: &str, extent: &[&str]) -> Parsed {
    let value = yaml_trim(rest);
    if !extent.is_empty() {
        if value.is_empty() {
            return list_extent(extent);
        }
        return Parsed::Opaque;
    }
    if value.is_empty() {
        return Parsed::Scalar(String::new());
    }
    scalar_or_inline(value)
}

/// Classifies an extent as a block-style list: every line one `- item`
/// at one shared indent. Anything else — a nested value, an interior
/// blank or comment, mixed indents — is outside the subset.
fn list_extent(extent: &[&str]) -> Parsed {
    let mut items = Vec::new();
    let mut indent = None;
    for line in extent {
        let trimmed = yaml_trim_start(line);
        let this_indent = &line[..line.len() - trimmed.len()];
        if this_indent.contains('\t') {
            return Parsed::Opaque;
        }
        match indent {
            None => indent = Some(this_indent),
            Some(seen) if seen == this_indent => {}
            Some(_) => return Parsed::Opaque,
        }
        let logical = if trimmed == "-" {
            String::new()
        } else if let Some(item) = trimmed.strip_prefix("- ")
            && let Some(logical) = plain_or_quoted(yaml_trim(item))
        {
            logical
        } else {
            return Parsed::Opaque;
        };
        items.push(logical);
    }
    Parsed::List(items)
}

/// Classifies an inline value: a flow list, a quoted or plain scalar, or
/// a shape outside the subset.
fn scalar_or_inline(value: &str) -> Parsed {
    if let Some(inner) = value.strip_prefix('[') {
        let Some(inner) = inner.strip_suffix(']') else {
            return Parsed::Opaque;
        };
        return match split_inline(inner) {
            Some(items) => Parsed::List(items),
            None => Parsed::Opaque,
        };
    }
    match plain_or_quoted(value) {
        Some(logical) => Parsed::Scalar(logical),
        None => Parsed::Opaque,
    }
}

/// Splits the interior of an inline `[...]` list into logical items.
/// `None` means a shape outside the subset: nesting, a malformed quote,
/// a dangling comma.
fn split_inline(inner: &str) -> Option<Vec<String>> {
    let mut items = Vec::new();
    let mut rest = yaml_trim_start(inner);
    if yaml_trim(rest).is_empty() {
        return Some(items);
    }
    loop {
        let (item, after) = if let Some(tail) = rest.strip_prefix('"') {
            let (logical, consumed) = scan_double(tail)?;
            (logical, &tail[consumed..])
        } else if let Some(tail) = rest.strip_prefix('\'') {
            let (logical, consumed) = scan_single(tail)?;
            (logical, &tail[consumed..])
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let piece = yaml_trim(&rest[..end]);
            if piece.is_empty()
                || piece.contains(['[', ']', '{', '}', '"', '\''])
                || needs_quotes(piece)
                || holds_forbidden_character(piece)
            {
                return None;
            }
            (piece.to_owned(), &rest[end..])
        };
        items.push(item);
        let after = yaml_trim_start(after);
        if after.is_empty() {
            return Some(items);
        }
        rest = yaml_trim_start(after.strip_prefix(',')?);
        if rest.is_empty() {
            return None;
        }
    }
}

/// Reads a double-quoted value from after its opening quote, returning
/// the logical value and the bytes consumed through the closing quote.
/// The full double-quote escape repertoire decodes — an escaped spelling
/// of a name must collide with its plain twin, never slip past it — but
/// a value that decodes to a forbidden control character stays out of
/// the subset like its raw form.
fn scan_double(tail: &str) -> Option<(String, usize)> {
    let mut logical = String::new();
    let mut chars = tail.char_indices();
    while let Some((index, character)) = chars.next() {
        match character {
            '"' => {
                return (!holds_forbidden_character(&logical)).then_some((logical, index + 1));
            }
            '\\' => match chars.next() {
                Some((_, '"')) => logical.push('"'),
                Some((_, '\\')) => logical.push('\\'),
                Some((_, '/')) => logical.push('/'),
                Some((_, ' ')) => logical.push(' '),
                Some((_, '0')) => logical.push('\0'),
                Some((_, 'a')) => logical.push('\u{7}'),
                Some((_, 'b')) => logical.push('\u{8}'),
                Some((_, 't')) => logical.push('\t'),
                Some((_, 'n')) => logical.push('\n'),
                Some((_, 'v')) => logical.push('\u{b}'),
                Some((_, 'f')) => logical.push('\u{c}'),
                Some((_, 'r')) => logical.push('\r'),
                Some((_, 'e')) => logical.push('\u{1b}'),
                Some((_, 'N')) => logical.push('\u{85}'),
                Some((_, '_')) => logical.push('\u{a0}'),
                Some((_, 'L')) => logical.push('\u{2028}'),
                Some((_, 'P')) => logical.push('\u{2029}'),
                Some((_, 'x')) => logical.push(hex_escape(&mut chars, 2)?),
                Some((_, 'u')) => logical.push(hex_escape(&mut chars, 4)?),
                Some((_, 'U')) => logical.push(hex_escape(&mut chars, 8)?),
                _ => return None,
            },
            other if forbidden_character(other) => return None,
            other => logical.push(other),
        }
    }
    None
}

/// Reads `digits` hex digits of a `\x`, `\u`, or `\U` escape and returns
/// the character they name, or `None` for a bad digit or a code point no
/// character has.
fn hex_escape(chars: &mut std::str::CharIndices<'_>, digits: u32) -> Option<char> {
    let mut code = 0;
    for _ in 0..digits {
        let (_, digit) = chars.next()?;
        code = code * 16 + digit.to_digit(16)?;
    }
    char::from_u32(code)
}

/// Reads a single-quoted value from after its opening quote; `''` is the
/// only escape.
fn scan_single(tail: &str) -> Option<(String, usize)> {
    let mut logical = String::new();
    let mut chars = tail.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        if character == '\'' {
            if matches!(chars.peek(), Some((_, '\''))) {
                logical.push('\'');
                chars.next();
            } else {
                return Some((logical, index + 1));
            }
        } else if forbidden_character(character) {
            return None;
        } else {
            logical.push(character);
        }
    }
    None
}

/// Reads a plain or quoted scalar as its logical value. `None` means a
/// shape outside the subset: a malformed or trailing-content quote, or a
/// plain spelling this module would not itself write. Accepting exactly
/// what [`needs_quotes`] would keep plain is what stops a nested value —
/// a mapping, a sequence, an anchor — from masquerading as text and
/// being rewritten into a string by a later edit.
fn plain_or_quoted(value: &str) -> Option<String> {
    if let Some(tail) = value.strip_prefix('"') {
        let (logical, consumed) = scan_double(tail)?;
        return (consumed == tail.len()).then_some(logical);
    }
    if let Some(tail) = value.strip_prefix('\'') {
        let (logical, consumed) = scan_single(tail)?;
        return (consumed == tail.len()).then_some(logical);
    }
    if needs_quotes(value) || holds_forbidden_character(value) {
        return None;
    }
    Some(value.to_owned())
}

/// Renders a logical value for writing, quoting only when the plain
/// spelling would read back as something else. `true`, `123`, or a date
/// stay plain on purpose: quoting them would change how an editor
/// types them.
fn rendered_scalar(value: &str) -> String {
    if needs_quotes(value) {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        value.to_owned()
    }
}

/// Whether a plain spelling of `value` would misread: as empty, as
/// trimmed, as YAML syntax, or as a comment. A tab always forces quotes,
/// because YAML treats it as separator whitespace: unquoted it can start
/// a comment or a mapping value mid-scalar. A dash is only syntax as a
/// bare `-` or a `- ` prefix, so a negative number stays plain and keeps
/// reading as a number.
fn needs_quotes(value: &str) -> bool {
    value.is_empty()
        || yaml_trim(value) != value
        || value.starts_with([
            '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@',
            '`',
        ])
        || value == "-"
        || value.starts_with("- ")
        || value.contains(": ")
        || value.ends_with(':')
        || value.contains(" #")
        || value.contains('\t')
}

/// A freshly rendered text property.
fn scalar_property(key: &str, value: &str, eol: &str) -> Property {
    Property {
        key: key.to_owned(),
        value: Parsed::Scalar(value.to_owned()),
        raw: vec![format!("{key}: {}{eol}", rendered_scalar(value))],
    }
}

/// A freshly rendered list property, in block style;
/// an empty list renders inline as `[]`, which still reads as a list.
fn list_property(key: &str, items: Vec<String>, eol: &str) -> Property {
    let mut raw = Vec::with_capacity(items.len() + 1);
    if items.is_empty() {
        raw.push(format!("{key}: []{eol}"));
    } else {
        raw.push(format!("{key}:{eol}"));
        for item in &items {
            raw.push(format!("  - {}{eol}", rendered_scalar(item)));
        }
    }
    Property {
        key: key.to_owned(),
        value: Parsed::List(items),
        raw,
    }
}

/// The value of `key` in the block, or an error naming why it cannot be
/// answered: only a unique property has one value.
fn current_value<'a>(block: &'a Block, key: &str) -> Result<Option<&'a Parsed>, Error> {
    let mut matches = block.items.iter().filter_map(|item| match item {
        Item::Property(property) if property.key == key => Some(&property.value),
        _ => None,
    });
    let first = matches.next();
    if matches.next().is_some() {
        return Err(Error::DuplicateKey {
            key: key.to_owned(),
        });
    }
    Ok(first)
}

/// The items of the list property `key`, `None` when it does not exist,
/// or an error when it exists as something other than a list.
fn current_list(block: &Block, key: &str) -> Result<Option<Vec<String>>, Error> {
    match current_value(block, key)? {
        None => Ok(None),
        Some(Parsed::List(items)) => Ok(Some(items.clone())),
        Some(Parsed::Scalar(_)) => Err(Error::NotAList {
            key: key.to_owned(),
        }),
        Some(Parsed::Opaque) => Err(Error::Opaque {
            key: key.to_owned(),
        }),
    }
}

/// Replaces the property sharing `new`'s key in place, or appends `new`
/// at the end of the block. Callers have already ruled out duplicates.
fn put(block: &mut Block, new: Property) {
    let position = block
        .items
        .iter()
        .position(|item| matches!(item, Item::Property(existing) if existing.key == new.key));
    match position {
        Some(index) => block.items[index] = Item::Property(new),
        None => block.items.push(Item::Property(new)),
    }
}

/// Reassembles a note from its parts; untouched parts are byte-exact.
fn render(document: &Document) -> String {
    let mut out = String::new();
    out.push_str(&document.head);
    if let Some(block) = &document.block {
        out.push_str(&block.opening);
        for item in &block.items {
            match item {
                Item::Property(property) => {
                    for line in &property.raw {
                        out.push_str(line);
                    }
                }
                Item::Raw(line) => out.push_str(line),
            }
        }
        out.push_str(&block.closing);
    }
    out.push_str(&document.body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(value: &str) -> Value {
        Value::Scalar(value.to_owned())
    }

    fn listed(items: &[&str]) -> Value {
        Value::List(items.iter().map(|&item| item.to_owned()).collect())
    }

    #[test]
    fn get_finds_nothing_without_a_block() {
        assert_eq!(get("body\n", "k").expect("get succeeds"), None);
        assert_eq!(get("", "k").expect("get succeeds"), None);
    }

    /// Measured empirically: a byte order mark before the fence is
    /// tolerated.
    #[test]
    fn get_tolerates_a_byte_order_mark() {
        assert_eq!(
            get("\u{feff}---\nk: v\n---\n", "k").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    /// Measured empirically: a blank line before the fence means no
    /// frontmatter.
    #[test]
    fn get_rejects_a_leading_blank_line() {
        assert_eq!(get("\n---\nk: v\n---\n", "k").expect("get succeeds"), None);
    }

    /// Measured empirically: CRLF fences and lines parse.
    #[test]
    fn get_reads_a_crlf_block() {
        assert_eq!(
            get("---\r\nk: v\r\n---\r\nbody\r\n", "k").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    /// Measured empirically: trailing spaces disqualify a fence.
    #[test]
    fn get_rejects_a_fence_with_trailing_space() {
        assert_eq!(get("--- \nk: v\n---\n", "k").expect("get succeeds"), None);
    }

    #[test]
    fn get_rejects_an_unclosed_fence() {
        assert_eq!(get("---\nk: v\nbody\n", "k").expect("get succeeds"), None);
        assert_eq!(get("---", "k").expect("get succeeds"), None);
    }

    /// Measured empirically: YAML's `...` does not close a block.
    #[test]
    fn get_rejects_a_dots_closer() {
        assert_eq!(get("---\nk: v\n...\n", "k").expect("get succeeds"), None);
    }

    #[test]
    fn get_rejects_a_fence_after_content() {
        assert_eq!(get("x\n---\nk: v\n---\n", "k").expect("get succeeds"), None);
    }

    #[test]
    fn get_misses_in_an_empty_block() {
        assert_eq!(get("---\n---\nbody\n", "k").expect("get succeeds"), None);
    }

    #[test]
    fn get_reads_a_scalar() {
        assert_eq!(
            get("---\nk: v\n---\n", "k").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    /// `tags: solo` reads as text, matching editors that keep
    /// property types outside the note.
    #[test]
    fn get_reads_a_scalar_tags_as_text() {
        assert_eq!(
            get("---\ntags: solo\n---\n", "tags").expect("get succeeds"),
            Some(scalar("solo"))
        );
    }

    #[test]
    fn get_trims_around_a_plain_scalar() {
        assert_eq!(
            get("---\nk:   v  \n---\n", "k").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    #[test]
    fn get_reads_an_empty_value_as_empty_text() {
        assert_eq!(
            get("---\nk:\n---\n", "k").expect("get succeeds"),
            Some(scalar(""))
        );
        assert_eq!(
            get("---\nk: \n---\n", "k").expect("get succeeds"),
            Some(scalar(""))
        );
    }

    #[test]
    fn get_unquotes_double_quotes() {
        assert_eq!(
            get("---\nk: \"a \\\"b\\\" \\\\ c\"\n---\n", "k").expect("get succeeds"),
            Some(scalar("a \"b\" \\ c"))
        );
    }

    #[test]
    fn get_unquotes_single_quotes() {
        assert_eq!(
            get("---\nk: 'it''s'\n---\n", "k").expect("get succeeds"),
            Some(scalar("it's"))
        );
    }

    #[test]
    fn get_reads_an_inline_list() {
        assert_eq!(
            get("---\nk: [a, \"b c\", 'd']\n---\n", "k").expect("get succeeds"),
            Some(listed(&["a", "b c", "d"]))
        );
    }

    #[test]
    fn get_reads_an_empty_inline_list() {
        assert_eq!(
            get("---\nk: []\n---\n", "k").expect("get succeeds"),
            Some(listed(&[]))
        );
        assert_eq!(
            get("---\nk: [ ]\n---\n", "k").expect("get succeeds"),
            Some(listed(&[]))
        );
    }

    #[test]
    fn get_reads_a_block_list() {
        assert_eq!(
            get("---\nk:\n  - a\n  - b c\n---\n", "k").expect("get succeeds"),
            Some(listed(&["a", "b c"]))
        );
    }

    #[test]
    fn get_reads_a_bare_dash_as_an_empty_item() {
        assert_eq!(
            get("---\nk:\n  - a\n  -\n---\n", "k").expect("get succeeds"),
            Some(listed(&["a", ""]))
        );
    }

    #[test]
    fn get_reads_a_zero_indent_list() {
        assert_eq!(
            get("---\nk:\n- a\n- b\n---\n", "k").expect("get succeeds"),
            Some(listed(&["a", "b"]))
        );
    }

    #[test]
    fn get_ignores_non_property_lines() {
        let text = "---\n# comment\n\n: v\nplain\nk:value\nk2: x\n---\n";
        assert_eq!(get(text, "k2").expect("get succeeds"), Some(scalar("x")));
        assert_eq!(get(text, "k").expect("get succeeds"), None);
    }

    #[test]
    fn get_reads_around_separating_lines() {
        let text = "---\na:\n  - x\n\n# note\nb: y\n---\n";
        assert_eq!(get(text, "a").expect("get succeeds"), Some(listed(&["x"])));
        assert_eq!(get(text, "b").expect("get succeeds"), Some(scalar("y")));
    }

    #[test]
    fn get_errors_on_duplicates() {
        let error = get("---\nk: a\nk: b\n---\n", "k").expect_err("duplicates fail");
        assert_eq!(error.to_string(), "multiple properties named \"k\"");
    }

    #[test]
    fn get_reads_past_other_keys_duplicates() {
        assert_eq!(
            get("---\nk: a\nk: b\nother: v\n---\n", "other").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    fn opaque_error(text: &str) {
        let error = get(text, "k").expect_err("out-of-subset value fails");
        assert_eq!(
            error.to_string(),
            "property \"k\" is not a text or list value"
        );
    }

    #[test]
    fn get_rejects_a_nested_mapping() {
        opaque_error("---\nk:\n  sub: x\n---\n");
    }

    /// A list item that is itself YAML structure — a mapping, a
    /// sequence, a flow collection — must stay opaque, or an edit would
    /// rewrite it as a quoted string.
    #[test]
    fn get_rejects_structured_list_items() {
        opaque_error("---\nk:\n  - name: Alice\n---\n");
        opaque_error("---\nk:\n  - - a\n---\n");
        opaque_error("---\nk:\n  - [a, b]\n---\n");
        opaque_error("---\nk: [a: b]\n---\n");
    }

    #[test]
    fn get_rejects_a_scalar_holding_a_mapping() {
        opaque_error("---\nk: a: b\n---\n");
        opaque_error("---\nk: ends:\n---\n");
    }

    /// YAML reads a tab as separator whitespace, so an unquoted tab
    /// cannot be trusted as part of a plain value.
    #[test]
    fn get_rejects_a_plain_value_with_a_tab() {
        opaque_error("---\nk: a\tb\n---\n");
        opaque_error("---\nk:\n  - a\tb\n---\n");
    }

    /// A control character has no writable spelling in the subset and
    /// could drive the reader's terminal, so it stays opaque however it
    /// is quoted.
    #[test]
    fn get_rejects_values_holding_control_characters() {
        opaque_error("---\nk: a\u{7}b\n---\n");
        opaque_error("---\nk: \"a\u{7}b\"\n---\n");
        opaque_error("---\nk: 'a\u{1b}[31mb'\n---\n");
    }

    /// A dash is only YAML syntax as a bare `-` or a `- ` prefix, so a
    /// negative number reads as the text it is and round-trips unquoted,
    /// keeping its number reading.
    #[test]
    fn get_reads_negative_numbers_as_plain_text() {
        assert_eq!(
            get("---\nk: -1\n---\n", "k").expect("get succeeds"),
            Some(scalar("-1"))
        );
        assert_eq!(
            get("---\nk:\n  - -1\n---\n", "k").expect("get succeeds"),
            Some(listed(&["-1"]))
        );
    }

    /// A column-zero line the parser does not read as a key — a stray
    /// word, a lone quoted scalar — is its own line, never part of the
    /// property above it, so editing that property leaves it untouched.
    #[test]
    fn edits_leave_unrecognized_neighbor_lines_alone() {
        let text = "---\nk: old\nplain stray\nnext: kept\n---\nbody\n";
        assert_eq!(get(text, "k").expect("get succeeds"), Some(scalar("old")));
        assert_eq!(
            set(text, "k", "new").expect("set succeeds"),
            "---\nk: new\nplain stray\nnext: kept\n---\nbody\n"
        );
        assert_eq!(
            unset(text, "k").expect("unset succeeds"),
            "---\nplain stray\nnext: kept\n---\nbody\n"
        );
        assert_eq!(
            get("---\nk: v\nstray\n---\n", "k").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    /// A quoted key names the same property as its plain spelling:
    /// lookups find it, an edit rewrites it in canonical plain form, and
    /// a plain twin collides as the duplicate it is.
    #[test]
    fn quoted_keys_name_the_property_they_spell() {
        let text = "---\n\"status\": old\n---\n";
        assert_eq!(
            get(text, "status").expect("get succeeds"),
            Some(scalar("old"))
        );
        assert_eq!(
            set(text, "status", "new").expect("set succeeds"),
            "---\nstatus: new\n---\n"
        );
        get("---\n\"status\": a\nstatus: b\n---\n", "status").expect_err("duplicates fail");
        assert_eq!(
            get("---\n'it''s': v\n---\n", "it's").expect("get succeeds"),
            Some(scalar("v"))
        );
        assert_eq!(
            get("---\n\"tags\":\n  - a\n---\n", "tags").expect("get succeeds"),
            Some(listed(&["a"]))
        );
        assert_eq!(
            get("---\n\"padded\" : v\n---\n", "padded").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    /// A quote without a colon directly after it is a value or a stray
    /// line, not a key: past the block's property root, it stays raw and
    /// edits leave it alone.
    #[test]
    fn quoted_lines_without_a_colon_stay_raw() {
        let text = "---\nfirst: p\n\"just a scalar\"\n\"\": odd\n\"q\" x: v\nk: v\n---\n";
        assert_eq!(get(text, "k").expect("get succeeds"), Some(scalar("v")));
        assert_eq!(
            set(text, "k", "w").expect("set succeeds"),
            "---\nfirst: p\n\"just a scalar\"\n\"\": odd\n\"q\" x: v\nk: w\n---\n"
        );
    }

    /// A list item can only continue a key line with no inline value; a
    /// dash line after a scalar is its own line and survives edits.
    #[test]
    fn edits_leave_a_dash_line_after_a_scalar_alone() {
        let text = "---\nk: old\n- stray\nnext: kept\n---\n";
        assert_eq!(get(text, "k").expect("get succeeds"), Some(scalar("old")));
        assert_eq!(
            set(text, "k", "new").expect("set succeeds"),
            "---\nk: new\n- stray\nnext: kept\n---\n"
        );
    }

    #[test]
    fn get_rejects_inline_items_holding_control_characters() {
        opaque_error("---\nk: [a\u{7}b]\n---\n");
    }

    /// YAML trims whitespace between a key and its colon, so kladde must
    /// find the property under its trimmed name.
    #[test]
    fn get_trims_whitespace_before_the_colon() {
        assert_eq!(
            get("---\nstatus : draft\n---\n", "status").expect("get succeeds"),
            Some(scalar("draft"))
        );
        get("---\nstatus : a\nstatus: b\n---\n", "status").expect_err("duplicates fail");
    }

    #[test]
    fn set_replaces_a_property_spelled_with_padding() {
        assert_eq!(
            set("---\nstatus : draft\n---\n", "status", "done").expect("set succeeds"),
            "---\nstatus: done\n---\n"
        );
    }

    #[test]
    fn get_rejects_a_scalar_with_continuation_lines() {
        opaque_error("---\nk: v\n  continued\n---\n");
    }

    #[test]
    fn get_rejects_block_scalars() {
        opaque_error("---\nk: |\n  text\n---\n");
        opaque_error("---\nk: >\n  text\n---\n");
        opaque_error("---\nk: |\n---\n");
    }

    #[test]
    fn get_rejects_an_inline_mapping() {
        opaque_error("---\nk: {a: b}\n---\n");
    }

    #[test]
    fn get_rejects_an_inline_comment() {
        opaque_error("---\nk: v # note\n---\n");
    }

    #[test]
    fn get_rejects_yaml_syntax_starts() {
        opaque_error("---\nk: &anchor\n---\n");
        opaque_error("---\nk: *alias\n---\n");
    }

    #[test]
    fn get_rejects_bad_escapes() {
        opaque_error("---\nk: \"a \\q\"\n---\n");
    }

    /// Every double-quote escape decodes, so an escaped spelling is the
    /// same logical text as its plain twin.
    #[test]
    fn double_quoted_escapes_decode() {
        let cases = [
            ("\\/", "/"),
            ("\\ ", " "),
            ("\\t", "\t"),
            ("\\_", "\u{a0}"),
            ("\\x41", "A"),
            ("\\u0064", "d"),
            ("\\U0001F4DD", "\u{1f4dd}"),
        ];
        for (escape, decoded) in cases {
            let text = format!("---\nk: \"a{escape}b\"\n---\n");
            assert_eq!(
                get(&text, "k").expect("get succeeds"),
                Some(scalar(&format!("a{decoded}b"))),
                "for {escape}"
            );
        }
    }

    /// An escape that decodes to a forbidden character is out of the
    /// subset, exactly like the raw form would be; the line and
    /// paragraph separators count, since older YAML readers break lines
    /// on them.
    #[test]
    fn control_escapes_stay_opaque() {
        for escape in [
            "\\0", "\\a", "\\b", "\\n", "\\v", "\\f", "\\r", "\\e", "\\N", "\\L", "\\P",
        ] {
            opaque_error(&format!("---\nk: \"a{escape}b\"\n---\n"));
        }
    }

    #[test]
    fn bad_hex_escapes_stay_opaque() {
        opaque_error("---\nk: \"\\uZZZZ\"\n---\n");
        opaque_error("---\nk: \"\\u00\"\n---\n");
        opaque_error("---\nk: \"\\uD800\"\n---\n");
    }

    /// An escaped spelling of a key collides with its plain twin instead
    /// of slipping past duplicate detection.
    #[test]
    fn escaped_quoted_keys_collide_with_their_plain_twin() {
        assert_eq!(
            get("---\n\"up\\u0064ated\": old\n---\n", "updated").expect("get succeeds"),
            Some(scalar("old"))
        );
        get("---\n\"up\\u0064ated\": a\nupdated: b\n---\n", "updated")
            .expect_err("duplicates fail");
        assert_eq!(
            set("---\n\"up\\u0064ated\": old\n---\n", "updated", "new").expect("set succeeds"),
            "---\nupdated: new\n---\n"
        );
    }

    #[test]
    fn get_rejects_unterminated_quotes() {
        opaque_error("---\nk: \"a\n---\n");
        opaque_error("---\nk: 'a\n---\n");
    }

    #[test]
    fn get_rejects_content_after_a_quote() {
        opaque_error("---\nk: \"a\" b\n---\n");
        opaque_error("---\nk: 'a' b\n---\n");
    }

    #[test]
    fn get_rejects_nested_inline_lists() {
        opaque_error("---\nk: [[a], b]\n---\n");
        opaque_error("---\nk: [a, {b: c}]\n---\n");
    }

    #[test]
    fn get_rejects_a_dangling_comma() {
        opaque_error("---\nk: [a,]\n---\n");
        opaque_error("---\nk: [a, , b]\n---\n");
    }

    #[test]
    fn get_rejects_an_unbalanced_bracket() {
        opaque_error("---\nk: [a\n---\n");
    }

    #[test]
    fn get_rejects_content_after_a_quoted_item() {
        opaque_error("---\nk: [\"a\" b]\n---\n");
    }

    #[test]
    fn get_rejects_mixed_list_indent() {
        opaque_error("---\nk:\n  - a\n    - b\n---\n");
    }

    #[test]
    fn get_rejects_an_interior_comment_in_a_list() {
        opaque_error("---\nk:\n  - a\n  # note\n  - b\n---\n");
    }

    #[test]
    fn get_rejects_an_interior_blank_in_a_list() {
        opaque_error("---\nk:\n  - a\n\n  - b\n---\n");
    }

    #[test]
    fn get_rejects_an_item_with_an_inline_comment() {
        opaque_error("---\nk:\n  - a # note\n---\n");
    }

    #[test]
    fn keys_are_validated_uniformly() {
        let cases = [
            ("", "it is empty"),
            ("a:b", "it contains a colon"),
            ("a#b", "it contains a hash"),
            ("a\nb", "it contains a line break"),
            ("a\tb", "it contains an unprintable character"),
            ("a\u{fffe}b", "it contains an unprintable character"),
            ("a\u{2028}b", "it contains an unprintable character"),
            (" a", "it has leading or trailing whitespace"),
            ("-a", "it starts with a character YAML reserves"),
            ("\"a", "it starts with a character YAML reserves"),
            ("@a", "it starts with a character YAML reserves"),
            ("*a", "it starts with a character YAML reserves"),
            ("[a", "it starts with a character YAML reserves"),
        ];
        for (key, reason) in cases {
            let error = get("---\n---\n", key).expect_err("invalid key fails");
            let message = error.to_string();
            assert!(message.starts_with("invalid property key"), "{message}");
            assert!(message.ends_with(reason), "{message}");
        }
    }

    #[test]
    fn values_must_be_single_lines() {
        let error = set("", "k", "a\nb").expect_err("multiline value fails");
        assert_eq!(
            error.to_string(),
            "property values are single lines, got \"a\\nb\""
        );
        add("", "k", "a\rb").expect_err("multiline item fails");
        remove("", "k", "a\nb").expect_err("multiline item fails");
    }

    /// A tab is the one control character a value may hold: it survives
    /// inside quotes, while the rest have no escape in the subset.
    #[test]
    fn values_cannot_hold_other_control_characters() {
        let error = set("", "k", "a\u{7}b").expect_err("control character fails");
        assert_eq!(
            error.to_string(),
            "property values cannot hold unprintable characters, got \"a\\u{7}b\""
        );
        add("", "k", "a\u{8}b").expect_err("control item fails");
        remove("", "k", "a\u{7}b").expect_err("control item fails");
        set("", "k", "a\tb").expect("a tab is allowed");
    }

    /// The two noncharacters YAML's printable set excludes, and the two
    /// separators older YAML readers break lines on, are rejected like
    /// controls — on the way in and on the way out.
    #[test]
    fn noncharacters_and_line_separators_are_forbidden() {
        set("", "k", "a\u{fffe}b").expect_err("noncharacter fails");
        add("", "k", "a\u{ffff}b").expect_err("noncharacter item fails");
        set("", "k", "a\u{2028}b").expect_err("line separator fails");
        set("", "k", "a\u{2029}b").expect_err("paragraph separator fails");
        opaque_error("---\nk: a\u{fffe}b\n---\n");
        opaque_error("---\nk: a\u{2028}b\n---\n");
        opaque_error("---\nk: \"\\uFFFF\"\n---\n");
    }

    /// A stamp cannot be built from parts a property could not hold, so
    /// holding one proves it renders cleanly.
    #[test]
    fn stamps_are_validated_at_construction() {
        Stamp::new("a:b", "updated", "T").expect_err("bad created key fails");
        Stamp::new("created", "-a", "T").expect_err("bad updated key fails");
        Stamp::new("created", "updated", "T\nx: y").expect_err("multiline timestamp fails");
    }

    /// A block rooted in anything but property lines — a flow value, a
    /// bare scalar, a block sequence — is preserved whole: writing
    /// property lines into it would corrupt the value, so edits refuse
    /// and stamping skips.
    #[test]
    fn foreign_rooted_blocks_are_never_edited_or_stamped() {
        let text = "---\n{foo: bar}\n---\nbody\n";
        let error = set(text, "k", "v").expect_err("set refuses");
        assert_eq!(
            error.to_string(),
            "the frontmatter is a single value, not properties"
        );
        add(text, "k", "a").expect_err("add refuses");
        unset(text, "k").expect_err("unset refuses");
        assert_eq!(stamped(text, true, &stamp()), text);
        let mixed = "---\n- item\nk: v\n---\n";
        unset(mixed, "k").expect_err("unset refuses a trailing property");
        remove(mixed, "k", "a").expect_err("remove refuses a trailing property");
        set("---\n[a, b]\n---\n", "k", "v").expect_err("flow sequence root refuses");
        set("---\n# c\n{foo: bar}\n---\n", "k", "v").expect_err("commented flow root refuses");
        let sequence = "---\n- a\n- b\n---\n";
        set(sequence, "k", "v").expect_err("block sequence root refuses");
        assert_eq!(stamped(sequence, true, &stamp()), sequence);
        let scalar_root = "---\njust a scalar\n---\n";
        add(scalar_root, "k", "a").expect_err("scalar root refuses");
        assert_eq!(stamped(scalar_root, true, &stamp()), scalar_root);
        let anchored = "---\n&props {foo: bar}\n---\n";
        set(anchored, "k", "v").expect_err("anchored flow root refuses");
        assert_eq!(stamped(anchored, true, &stamp()), anchored);
        set("---\n!!map {foo: bar}\n---\n", "k", "v").expect_err("tagged flow root refuses");
    }

    /// Commentary never decides the root, whatever its indent: a block
    /// opening with an indented comment is still a property block.
    #[test]
    fn indented_root_comments_do_not_make_a_block_foreign() {
        assert_eq!(
            set("---\n  # note\na: x\n---\n", "k", "v").expect("set succeeds"),
            "---\n  # note\na: x\nk: v\n---\n"
        );
    }

    /// An indented comment after a completed value is a standalone
    /// comment, not the property's content: edits leave it in place.
    /// Only a value-bearing extent, like a block scalar, owns one.
    #[test]
    fn indented_comments_after_a_value_stand_alone() {
        let text = "---\nk: old\n  # keep\nnext: v\n---\n";
        assert_eq!(get(text, "k").expect("get succeeds"), Some(scalar("old")));
        assert_eq!(
            set(text, "k", "new").expect("set succeeds"),
            "---\nk: new\n  # keep\nnext: v\n---\n"
        );
        assert_eq!(
            unset(text, "k").expect("unset succeeds"),
            "---\n  # keep\nnext: v\n---\n"
        );
    }

    /// A no-break space is content, not separation: a key ending in one
    /// is a different key, a line starting with one is its own line, and
    /// a value keeps its wide whitespace through a round trip.
    #[test]
    fn unicode_whitespace_is_content_not_separation() {
        let text = "---\na\u{a0}: kept\n---\n";
        assert_eq!(get(text, "a").expect("get succeeds"), None);
        assert_eq!(
            set(text, "a", "v").expect("set succeeds"),
            "---\na\u{a0}: kept\na: v\n---\n"
        );
        let neighbor = "---\nk: v\n\u{a0}x: y\n---\n";
        assert_eq!(get(neighbor, "k").expect("get succeeds"), Some(scalar("v")));
        assert_eq!(
            set(neighbor, "k", "w").expect("set succeeds"),
            "---\nk: w\n\u{a0}x: y\n---\n"
        );
        assert_eq!(
            get("---\nk: \u{a0}v\u{a0}\n---\n", "k").expect("get succeeds"),
            Some(scalar("\u{a0}v\u{a0}"))
        );
        let wide = "---\nk: \u{a0}\n- raw\n---\n";
        assert_eq!(
            get(wide, "k").expect("get succeeds"),
            Some(scalar("\u{a0}"))
        );
        assert_eq!(
            set(wide, "k", "new").expect("set succeeds"),
            "---\nk: new\n- raw\n---\n"
        );
    }

    /// YAML rejects a tab as indentation, so a tab-indented extent is
    /// outside the subset.
    #[test]
    fn get_rejects_tab_indented_lists() {
        opaque_error("---\nk:\n\t- a\n---\n");
    }

    /// A trailing indented comment belongs to the extent above it, where
    /// a block scalar owns it as content: edits take it with the
    /// property instead of leaving half the value behind.
    #[test]
    fn block_scalar_content_stays_with_its_property() {
        assert_eq!(
            set("---\nk: |\n  value\n  # literal\n---\n", "k", "new").expect("set succeeds"),
            "---\nk: new\n---\n"
        );
        assert_eq!(
            unset("---\nk: |\n  value\n  # literal\n---\nbody\n", "k").expect("unset succeeds"),
            "body\n"
        );
        opaque_error("---\nk:\n  - a\n  # trailing\n---\n");
    }

    /// YAML reads a tab after the colon as the separator, exactly like a
    /// space: the property is visible to lookups, duplicate detection,
    /// and edits, which rewrite it canonically.
    #[test]
    fn tabs_after_the_colon_separate_like_spaces() {
        assert_eq!(
            get("---\nk:\tv\n---\n", "k").expect("get succeeds"),
            Some(scalar("v"))
        );
        get("---\nk:\ta\nk: b\n---\n", "k").expect_err("duplicates fail");
        assert_eq!(
            set("---\nk:\told\n---\n", "k", "new").expect("set succeeds"),
            "---\nk: new\n---\n"
        );
        assert_eq!(
            get("---\n\"k\":\tv\n---\n", "k").expect("get succeeds"),
            Some(scalar("v"))
        );
    }

    #[test]
    fn set_creates_a_block_on_an_empty_note() {
        assert_eq!(set("", "k", "v").expect("set succeeds"), "---\nk: v\n---\n");
    }

    #[test]
    fn set_creates_a_block_above_a_body() {
        assert_eq!(
            set("body\n", "k", "v").expect("set succeeds"),
            "---\nk: v\n---\nbody\n"
        );
    }

    #[test]
    fn set_prepends_above_a_body_starting_with_a_fence() {
        assert_eq!(
            set("---\nnot closed\n", "k", "v").expect("set succeeds"),
            "---\nk: v\n---\n---\nnot closed\n"
        );
    }

    /// Measured empirically: an empty block is recognized as
    /// frontmatter, so a property lands inside it, never in a second
    /// block.
    #[test]
    fn set_reuses_an_empty_block() {
        assert_eq!(
            set("---\n---\nbody\n", "k", "v").expect("set succeeds"),
            "---\nk: v\n---\nbody\n"
        );
    }

    #[test]
    fn set_replaces_in_place_and_preserves_neighbors() {
        let text = "---\na: 'kept'\n# note\nk: old\nz:\n  - kept\n---\nbody\n";
        assert_eq!(
            set(text, "k", "new").expect("set succeeds"),
            "---\na: 'kept'\n# note\nk: new\nz:\n  - kept\n---\nbody\n"
        );
    }

    #[test]
    fn set_overwrites_a_list() {
        assert_eq!(
            set("---\nk:\n  - a\n---\n", "k", "v").expect("set succeeds"),
            "---\nk: v\n---\n"
        );
    }

    #[test]
    fn set_overwrites_an_opaque_value() {
        assert_eq!(
            set("---\nk:\n  sub: x\n---\n", "k", "v").expect("set succeeds"),
            "---\nk: v\n---\n"
        );
    }

    #[test]
    fn set_appends_a_new_property_at_the_end() {
        assert_eq!(
            set("---\na: x\n---\n", "b", "y").expect("set succeeds"),
            "---\na: x\nb: y\n---\n"
        );
    }

    #[test]
    fn set_errors_on_duplicates() {
        set("---\nk: a\nk: b\n---\n", "k", "v").expect_err("duplicates fail");
    }

    #[test]
    fn set_quotes_only_when_needed() {
        let cases = [
            ("", "k: \"\""),
            (" padded ", "k: \" padded \""),
            ("[[Link]]", "k: \"[[Link]]\""),
            ("a: b", "k: \"a: b\""),
            ("ends:", "k: \"ends:\""),
            ("a #b", "k: \"a #b\""),
            ("-x", "k: -x"),
            ("-1", "k: -1"),
            ("-", "k: \"-\""),
            ("- x", "k: \"- x\""),
            ("a\tb", "k: \"a\tb\""),
            ("\u{a0}v\u{a0}", "k: \u{a0}v\u{a0}"),
            ("a\"b\\c", "k: a\"b\\c"),
            ("x: a\"b\\c", "k: \"x: a\\\"b\\\\c\""),
            ("true", "k: true"),
            ("123", "k: 123"),
            ("2026-08-05", "k: 2026-08-05"),
            ("plain text", "k: plain text"),
        ];
        for (value, line) in cases {
            let text = set("", "k", value).expect("set succeeds");
            assert_eq!(text, format!("---\n{line}\n---\n"));
            assert_eq!(get(&text, "k").expect("get succeeds"), Some(scalar(value)));
        }
    }

    #[test]
    fn set_keeps_a_crlf_block_crlf() {
        let text = "---\r\na: x\r\n---\r\nbody\r\n";
        assert_eq!(
            set(text, "k", "v").expect("set succeeds"),
            "---\r\na: x\r\nk: v\r\n---\r\nbody\r\n"
        );
    }

    #[test]
    fn set_keeps_a_byte_order_mark() {
        assert_eq!(
            set("\u{feff}---\na: x\n---\n", "k", "v").expect("set succeeds"),
            "\u{feff}---\na: x\nk: v\n---\n"
        );
    }

    /// A new block lands after the byte order mark, never above it: a
    /// BOM belongs at byte zero, and a fence still parses after one.
    #[test]
    fn set_creates_the_block_after_a_byte_order_mark() {
        assert_eq!(
            set("\u{feff}body\n", "k", "v").expect("set succeeds"),
            "\u{feff}---\nk: v\n---\nbody\n"
        );
    }

    #[test]
    fn unset_removes_the_property() {
        assert_eq!(
            unset("---\na: x\nk: v\n---\nbody\n", "k").expect("unset succeeds"),
            "---\na: x\n---\nbody\n"
        );
    }

    #[test]
    fn unset_removes_the_fences_with_the_last_property() {
        assert_eq!(
            unset("---\nk: v\n---\nbody\n", "k").expect("unset succeeds"),
            "body\n"
        );
    }

    #[test]
    fn unset_keeps_fences_holding_other_lines() {
        assert_eq!(
            unset("---\n# note\nk: v\n---\n", "k").expect("unset succeeds"),
            "---\n# note\n---\n"
        );
    }

    /// Dropping the fences must never promote body text into
    /// frontmatter: a body that reads as a block keeps an empty block
    /// fencing it off.
    #[test]
    fn unset_keeps_fences_when_the_body_reads_as_a_block() {
        assert_eq!(
            unset("---\nk: v\n---\n---\nevil: y\n---\nbody\n", "k").expect("unset succeeds"),
            "---\n---\n---\nevil: y\n---\nbody\n"
        );
        assert_eq!(
            get(
                &unset("---\nk: v\n---\n---\nevil: y\n---\nbody\n", "k").expect("unset succeeds"),
                "evil"
            )
            .expect("get succeeds"),
            None
        );
    }

    #[test]
    fn unset_of_a_missing_property_changes_nothing() {
        let text = "---\na: x\n---\nweird  bytes\n";
        assert_eq!(unset(text, "k").expect("unset succeeds"), text);
        assert_eq!(
            unset("no block\n", "k").expect("unset succeeds"),
            "no block\n"
        );
    }

    #[test]
    fn unset_errors_on_duplicates() {
        unset("---\nk: a\nk: b\n---\n", "k").expect_err("duplicates fail");
    }

    #[test]
    fn add_creates_the_block_and_the_list() {
        assert_eq!(
            add("", "k", "a").expect("add succeeds"),
            "---\nk:\n  - a\n---\n"
        );
    }

    #[test]
    fn add_appends_to_an_existing_list() {
        assert_eq!(
            add("---\nk:\n  - a\n---\n", "k", "b").expect("add succeeds"),
            "---\nk:\n  - a\n  - b\n---\n"
        );
    }

    #[test]
    fn add_rewrites_an_inline_list_in_block_style() {
        assert_eq!(
            add("---\nk: [a]\n---\n", "k", "b").expect("add succeeds"),
            "---\nk:\n  - a\n  - b\n---\n"
        );
    }

    #[test]
    fn add_fills_an_empty_list() {
        assert_eq!(
            add("---\nk: []\n---\n", "k", "a").expect("add succeeds"),
            "---\nk:\n  - a\n---\n"
        );
    }

    #[test]
    fn add_of_a_present_item_changes_nothing() {
        let text = "---\nk: [a]\n---\n";
        assert_eq!(add(text, "k", "a").expect("add succeeds"), text);
    }

    #[test]
    fn add_quotes_items_that_need_it() {
        assert_eq!(
            add("", "k", "[[Link]]").expect("add succeeds"),
            "---\nk:\n  - \"[[Link]]\"\n---\n"
        );
    }

    #[test]
    fn add_rejects_a_text_property() {
        let error = add("---\nk: v\n---\n", "k", "a").expect_err("text property fails");
        assert_eq!(error.to_string(), "property \"k\" is not a list");
    }

    #[test]
    fn add_rejects_an_opaque_property() {
        add("---\nk:\n  sub: x\n---\n", "k", "a").expect_err("opaque fails");
    }

    #[test]
    fn add_errors_on_duplicates() {
        add("---\nk: [a]\nk: [b]\n---\n", "k", "c").expect_err("duplicates fail");
    }

    #[test]
    fn remove_removes_an_item() {
        assert_eq!(
            remove("---\nk:\n  - a\n  - b\n---\n", "k", "a").expect("remove succeeds"),
            "---\nk:\n  - b\n---\n"
        );
    }

    #[test]
    fn remove_of_the_last_item_leaves_an_empty_list() {
        assert_eq!(
            remove("---\nk:\n  - a\n---\n", "k", "a").expect("remove succeeds"),
            "---\nk: []\n---\n"
        );
    }

    #[test]
    fn remove_of_an_absent_item_changes_nothing() {
        let text = "---\nk:\n  - a\n---\n";
        assert_eq!(remove(text, "k", "b").expect("remove succeeds"), text);
        assert_eq!(
            remove("---\nk: []\n---\n", "k", "a").expect("remove succeeds"),
            "---\nk: []\n---\n"
        );
    }

    #[test]
    fn remove_of_a_missing_property_changes_nothing() {
        assert_eq!(
            remove("body\n", "k", "a").expect("remove succeeds"),
            "body\n"
        );
        assert_eq!(
            remove("---\na: x\n---\n", "k", "a").expect("remove succeeds"),
            "---\na: x\n---\n"
        );
    }

    #[test]
    fn remove_rejects_a_text_property() {
        remove("---\nk: v\n---\n", "k", "a").expect_err("text property fails");
    }

    fn stamp() -> Stamp<'static> {
        Stamp::new(DEFAULT_CREATED_KEY, DEFAULT_UPDATED_KEY, "T").expect("stamp validates")
    }

    #[test]
    fn has_block_detects_a_block() {
        assert!(has_block("---\n---\n"));
        assert!(!has_block("body\n"));
    }

    #[test]
    fn stamped_creates_a_block_with_both_stamps() {
        assert_eq!(
            stamped("- x\n", false, &stamp()),
            "---\ncreated: T\nupdated: T\n---\n- x\n"
        );
    }

    #[test]
    fn stamped_only_updates_an_existing_block() {
        assert_eq!(
            stamped("---\nk: v\nupdated: old\n---\n", true, &stamp()),
            "---\nk: v\nupdated: T\n---\n"
        );
        assert_eq!(
            stamped("---\nk: v\n---\n", true, &stamp()),
            "---\nk: v\nupdated: T\n---\n"
        );
    }

    /// A created stamp the same edit set by hand survives: the automatic
    /// stamp only fills the key in when it is absent.
    #[test]
    fn stamped_preserves_a_manual_created_value() {
        assert_eq!(
            stamped("---\ncreated: mine\n---\n", false, &stamp()),
            "---\ncreated: mine\nupdated: T\n---\n"
        );
    }

    #[test]
    fn stamped_never_resurrects_a_removed_block() {
        assert_eq!(stamped("body\n", true, &stamp()), "body\n");
    }

    #[test]
    fn stamped_skips_notes_whose_stamp_keys_misbehave() {
        let duplicated = "---\nupdated: a\nupdated: b\n---\n";
        assert_eq!(stamped(duplicated, true, &stamp()), duplicated);
        let listed = "---\nupdated:\n  - a\n---\n";
        assert_eq!(stamped(listed, true, &stamp()), listed);
    }

    /// A quoted or tab-separated spelling of the updated key is the same
    /// property, so the stamp replaces it instead of writing a duplicate
    /// beside it.
    #[test]
    fn stamped_replaces_alternate_updated_spellings() {
        assert_eq!(
            stamped("---\n\"updated\": old\nk: v\n---\n", true, &stamp()),
            "---\nupdated: T\nk: v\n---\n"
        );
        assert_eq!(
            stamped("---\nupdated:\told\n---\n", true, &stamp()),
            "---\nupdated: T\n---\n"
        );
    }

    /// A misbehaving created key skips the whole stamp, updated
    /// included: stamping is all or nothing.
    #[test]
    fn stamped_skips_wholly_when_the_created_key_misbehaves() {
        let listed = "---\ncreated:\n  - mine\n---\n";
        assert_eq!(stamped(listed, false, &stamp()), listed);
        let duplicated = "---\ncreated: a\ncreated: b\n---\n";
        assert_eq!(stamped(duplicated, false, &stamp()), duplicated);
    }

    #[test]
    fn stamped_keeps_a_byte_order_mark_at_byte_zero() {
        assert_eq!(
            stamped("\u{feff}- x\n", false, &stamp()),
            "\u{feff}---\ncreated: T\nupdated: T\n---\n- x\n"
        );
    }

    #[test]
    fn stamped_writes_one_line_when_the_keys_coincide() {
        let one = Stamp::new("touched", "touched", "T").expect("stamp validates");
        assert_eq!(stamped("- x\n", false, &one), "---\ntouched: T\n---\n- x\n");
    }

    /// Every canonical spelling survives parse and render byte for byte,
    /// and every logical value survives a write and a read: the
    /// composition of shapes and neighbors is generated rather than
    /// enumerated.
    #[test]
    fn canonical_documents_round_trip() {
        let scalars = ["plain", "", "a: b", " padded ", "tr\"icky\\", "true"];
        let lists: [&[&str]; 3] = [&["one"], &["a", "b c", "[[link]]"], &[]];
        let neighbors = ["", "# note\n", "\n", "deep:\n  sub: x\n"];
        for neighbor in neighbors {
            let base = if neighbor.is_empty() {
                String::new()
            } else {
                format!("---\n{neighbor}---\nbody\n")
            };
            for value in scalars {
                let text = set(&base, "k", value).expect("set succeeds");
                assert_eq!(render(&parse(&text)), text, "in {text:?}");
                assert_eq!(get(&text, "k").expect("get succeeds"), Some(scalar(value)));
                assert_eq!(set(&text, "k", value).expect("set succeeds"), text);
            }
            for items in lists {
                let mut text = add(&base, "k", "seed").expect("add succeeds");
                for item in items {
                    text = add(&text, "k", item).expect("add succeeds");
                }
                text = remove(&text, "k", "seed").expect("remove succeeds");
                assert_eq!(render(&parse(&text)), text, "in {text:?}");
                assert_eq!(get(&text, "k").expect("get succeeds"), Some(listed(items)));
            }
        }
    }
}
