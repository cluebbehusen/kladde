//! Locating markdown structure — headings, sections, and bullet threads —
//! so an append can land inside a note instead of at its end. Pure text
//! in, text out, like [`crate::frontmatter`]: no I/O, environment, or
//! clock.
//!
//! Structure is what pulldown-cmark reports, so pseudo-structure inside
//! fenced or indented code, HTML, or blockquotes can never match, and a
//! matched target resolves to exactly one place: zero or several matches
//! is an error, never a silent fallback. The note's frontmatter block is
//! sliced off before parsing, because a `key: value` line above a
//! closing `---` fence would otherwise read as a setext heading.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::frontmatter;

/// Where inside a note an appended entry lands: a chain of headings
/// narrowing to a section, then a chain of bullets descending a thread
/// inside it. Both chains empty places the entry verbatim at the very
/// end of the note.
#[derive(Debug, Default)]
pub struct Placement {
    /// Heading queries, outermost first: each a prefix of the heading's
    /// text as written, its `#` marks excluded.
    pub headings: Vec<String>,
    /// Bullet queries, outermost first: each a prefix of a bullet's
    /// first line past its marker.
    pub bullets: Vec<String>,
    /// Indent unit under a childless bullet.
    pub indent: Indent,
}

/// The indent unit written for an entry nested under a bullet that has no
/// child bullet yet: the `bullet-indent` config key. A bullet that
/// already has a child copies the child's indent instead, because a
/// note's own style beats configuration.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Indent {
    /// The bullet's own leading whitespace plus tabs enough to reach the
    /// column where the bullet's text starts, the indent `CommonMark`
    /// asks of a nested item.
    #[default]
    Tab,
    /// Spaces up to that same column.
    Spaces,
}

impl Indent {
    /// The spelling the config file stores.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::Spaces => "spaces",
        }
    }
}

/// Failure while locating a structural target in a note.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid heading \"{}\": {reason}", query.escape_debug())]
    InvalidHeading { query: String, reason: &'static str },
    #[error("invalid bullet \"{}\": {reason}", query.escape_debug())]
    InvalidBullet { query: String, reason: &'static str },
    #[error("no heading matching \"{}\"", query.escape_debug())]
    NoHeading { query: String },
    #[error("multiple headings matching \"{}\"", query.escape_debug())]
    AmbiguousHeading { query: String },
    #[error("no bullet matching \"{}\"", query.escape_debug())]
    NoBullet { query: String },
    #[error("multiple bullets matching \"{}\"", query.escape_debug())]
    AmbiguousBullet { query: String },
    #[error("the target ends inside an unclosed code fence")]
    UnclosedFence,
    #[error("the target ends inside unclosed raw HTML")]
    UnclosedHtml,
    #[error("the note uses bare carriage-return line endings")]
    BareCarriageReturn,
    #[error("the entry would change the structure around it")]
    AbsorbedStructure,
    #[error("the bullet matching \"{}\" holds nested content", query.escape_debug())]
    NestedContent { query: String },
    #[error("the removal would change the structure around it")]
    RemovalReshapes,
    #[error("no task matching \"{}\"", query.escape_debug())]
    NoTask { query: String },
    #[error("multiple tasks matching \"{}\"", query.escape_debug())]
    AmbiguousTask { query: String },
    #[error("the bullet matching \"{}\" shares its line with another bullet", query.escape_debug())]
    SharedLine { query: String },
}

/// `text` with `entry` landed at the place `placement` names: the end of
/// the innermost matched section, or nested at the end of the matched
/// bullet's thread. Entry lines are terminated like their new neighbors,
/// so a CRLF note stays CRLF throughout. An empty placement appends the
/// entry verbatim at the very end of the note instead.
///
/// # Errors
///
/// Returns an error when a query is empty or spans lines, when a query
/// matches no heading or bullet in its scope or matches more than one,
/// when the entry would land inside an unclosed code fence or unclosed
/// raw HTML, which no entry can be kept out of, or when the note uses
/// bare carriage-return line endings, which the parser reads
/// inconsistently.
pub fn inserted(text: &str, entry: &str, placement: &Placement) -> Result<String, Error> {
    if placement.headings.is_empty() && placement.bullets.is_empty() {
        return Ok(appended(text, entry));
    }
    for query in &placement.headings {
        validated(query, Kind::Heading)?;
    }
    for query in &placement.bullets {
        validated(query, Kind::Bullet)?;
    }
    if bare_carriage_return(text) {
        return Err(Error::BareCarriageReturn);
    }
    let body_start = frontmatter::body_start(text);
    let body = &text[body_start..];
    let outline = outline(body);
    let scope = heading_scope(body, &outline, &placement.headings)?;
    let (at, indent) = if placement.bullets.is_empty() {
        (insertion_at(body, &scope), String::new())
    } else {
        let index = matched_bullet(body, &outline, scope, &placement.bullets)?;
        (
            insertion_at(body, &outline.bullets[index].span),
            entry_indent(body, &outline, index, placement.indent),
        )
    };
    let entry_columns = columns(&indent);
    outside_fences(body, &outline, at, entry_columns)?;
    let (at, terminated) = past_blocks(body, &outline, at, entry_columns)?;
    let eol = insertion_eol(&text[..body_start + at]);
    // The splice must leave the surroundings meaning what they meant: a
    // list entry directly above a setext heading would absorb it, plain
    // text directly below a list would be absorbed by it, and a heading
    // entry could reparent every section after it. Where a blank line —
    // neutral whitespace — restores the boundary it is written, on
    // either side or both; where none can, the placement is refused.
    let splice = Splice {
        before: blocks(body),
        at,
        boundary: body_start,
        old_len: body.len(),
        // An entry the parser renders invisible, like a link reference
        // definition, has no block to own; it only has to sit outside
        // every neighbor.
        owning: !blocks(entry).is_empty(),
        indent: indent.len(),
        // The separator a splice writes before the entry at an
        // unterminated line shifts where the entry itself begins.
        shift: if text[..body_start + at].ends_with('\n') {
            0
        } else {
            eol.len()
        },
        eol,
    };
    let base = rendered(entry, &indent, eol);
    let variants = [
        (terminated, false),
        (true, false),
        (terminated, true),
        (true, true),
    ];
    for (prefixed, suffixed) in variants {
        let lines = assembled(&base, prefixed, suffixed, eol);
        let candidate = spliced(text, body_start + at, &lines, eol);
        let (kept, own) = splice_check(&candidate, &splice, prefixed);
        if kept && own {
            return Ok(candidate);
        }
    }
    Err(Error::AbsorbedStructure)
}

/// The entry's rendered lines with the blank lines a splice asked for:
/// one before, one after, neither, or both.
fn assembled(base: &str, prefixed: bool, suffixed: bool, eol: &str) -> String {
    let mut lines = String::with_capacity(base.len() + 2 * eol.len());
    if prefixed {
        lines.push_str(eol);
    }
    lines.push_str(base);
    if suffixed {
        lines.push_str(eol);
    }
    lines
}

/// The block-level kinds a splice must not disturb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Paragraph,
    Heading(HeadingLevel),
    List(Option<u64>),
    Item,
    Code,
    Html,
    Quote,
    Rule,
}

/// Every block of the body, as kind, container depth, and span, in
/// document order: the snapshot a splice is checked against. Depth
/// matters as much as position — a heading captured into a list item
/// keeps its offset but gains a container.
fn blocks(body: &str) -> Vec<(BlockKind, usize, Range<usize>)> {
    let mut depth = 0_usize;
    let mut list = Vec::new();
    for (event, span) in Parser::new(body).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                let kind = match tag {
                    Tag::Paragraph => BlockKind::Paragraph,
                    Tag::Heading { level, .. } => BlockKind::Heading(level),
                    Tag::List(start) => BlockKind::List(start),
                    Tag::Item => BlockKind::Item,
                    Tag::CodeBlock(_) => BlockKind::Code,
                    Tag::HtmlBlock => BlockKind::Html,
                    Tag::BlockQuote(_) => BlockKind::Quote,
                    _ => continue,
                };
                list.push((kind, depth, span));
                depth += 1;
            }
            Event::Rule => list.push((BlockKind::Rule, depth, span)),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::HtmlBlock
                | TagEnd::BlockQuote(_),
            ) => depth -= 1,
            _ => {}
        }
    }
    list
}

/// What a splice is checked against: the original body's blocks and the
/// geometry of the insertion.
struct Splice<'a> {
    before: Vec<(BlockKind, usize, Range<usize>)>,
    at: usize,
    boundary: usize,
    old_len: usize,
    owning: bool,
    indent: usize,
    shift: usize,
    eol: &'a str,
}

/// Checks a spliced note against the blocks of the original body. The
/// surroundings must survive: blocks before the splice unchanged,
/// blocks after it intact at their shifted offsets, blocks holding the
/// splice point allowed to grow around it — except a paragraph ending
/// exactly there, which growing means absorbing the entry — and every
/// pre-existing heading keeping the parent heading it had. The entry
/// must also begin a block of its own at its first line rather than
/// continue a neighbor. Returns (surroundings kept, entry its own).
fn splice_check(new: &str, splice: &Splice<'_>, prefixed: bool) -> (bool, bool) {
    // An entry that completes a half-open frontmatter fence moves the
    // body boundary itself; nothing behind it can be trusted.
    if frontmatter::body_start(new) != splice.boundary {
        return (false, false);
    }
    let body = &new[splice.boundary..];
    let added = body.len() - splice.old_len;
    let at = splice.at;
    let after = blocks(body);
    let mut kept = true;
    let mut cursor = 0;
    'blocks: for (kind, depth, span) in &splice.before {
        let (start, end, holds) = if span.end < at {
            (span.start, span.end, false)
        } else if span.start >= at {
            (span.start + added, span.end + added, false)
        } else {
            (span.start, span.end, true)
        };
        while cursor < after.len() {
            let (other, nested, candidate) = &after[cursor];
            cursor += 1;
            // A holding paragraph may only gain the terminator the
            // splice wrote for an unterminated line: more means it
            // absorbed the entry. Other holding blocks grow around the
            // entry freely; everything else matches exactly.
            let end_ok = if !holds {
                candidate.end == end
            } else if *kind == BlockKind::Paragraph && span.end == at {
                candidate.end >= end && candidate.end <= end + splice.eol.len()
            } else {
                true
            };
            if other == kind && nested == depth && candidate.start == start && end_ok {
                continue 'blocks;
            }
            if candidate.start > start {
                break;
            }
        }
        kept = false;
        break;
    }
    let kept = kept && parents_kept(&splice.before, &after, at, added);
    // The entry's own block must anchor at its first line: at the line
    // start, within its indent, or one terminator back — a tab-nested
    // item's measured span can reach into the terminator before it.
    let entry = at + splice.shift + if prefixed { splice.eol.len() } else { 0 };
    let own = !splice.owning
        || after.iter().any(|(_, _, span)| {
            span.start + splice.eol.len() >= entry && span.start <= entry + splice.indent
        });
    (kept, own)
}

/// Whether every pre-existing heading keeps the parent heading it had:
/// an entry carrying a heading of its own can otherwise reparent every
/// section after it while each heading stays a heading.
fn parents_kept(
    before: &[(BlockKind, usize, Range<usize>)],
    after: &[(BlockKind, usize, Range<usize>)],
    at: usize,
    added: usize,
) -> bool {
    let expected: Vec<(HeadingLevel, usize)> = before
        .iter()
        .filter_map(|(kind, _, span)| {
            let BlockKind::Heading(level) = kind else {
                return None;
            };
            let start = if span.start >= at {
                span.start + added
            } else {
                span.start
            };
            Some((*level, start))
        })
        .collect();
    let found: Vec<(HeadingLevel, usize)> = after
        .iter()
        .filter_map(|(kind, _, span)| {
            let BlockKind::Heading(level) = kind else {
                return None;
            };
            Some((*level, span.start))
        })
        .collect();
    let wanted = parented(&expected);
    let real = parented(&found);
    wanted
        .iter()
        .all(|(start, parent)| real.iter().any(|pair| pair == &(*start, *parent)))
}

/// Whether a surviving block keeps its kind. An ordered list whose
/// first item was cut takes its start from the next item's marker, so
/// its survivors renumber downward or hold, the way deleting from a
/// numbered list does everywhere; only a raised number reshapes.
fn kind_kept(before: BlockKind, after: BlockKind, first_cut: bool) -> bool {
    match (before, after) {
        (BlockKind::List(Some(start)), BlockKind::List(Some(new))) if first_cut => {
            new <= start.saturating_add(1)
        }
        _ => before == after,
    }
}

/// Each heading's parent: the start of the nearest earlier heading with
/// a shallower level, or none at the top of the outline.
fn parented(headings: &[(HeadingLevel, usize)]) -> Vec<(usize, Option<usize>)> {
    let mut stack: Vec<(HeadingLevel, usize)> = Vec::new();
    let mut parents = Vec::new();
    for &(level, start) in headings {
        while stack.last().is_some_and(|&(above, _)| above >= level) {
            stack.pop();
        }
        parents.push((start, stack.last().map(|&(_, parent)| parent)));
        stack.push((level, start));
    }
    parents
}

/// The note with `entry` appended verbatim at its very end: one
/// terminating newline is written, with a separating one first when the
/// note does not already end in one.
fn appended(current: &str, entry: &str) -> String {
    let mut new = String::with_capacity(current.len() + entry.len() + 2);
    new.push_str(current);
    if !current.is_empty() && !current.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(entry);
    new.push('\n');
    new
}

/// `text` with the one bullet `query` names removed: its own line cut
/// out, never its thread. The bullet is matched inside the scope the
/// placement names, by a prefix of its first line past its marker, the
/// same contract an appended entry's target follows; the placement's
/// indent is insertion's concern and is ignored here. A bullet holding
/// content beyond its first line, or another addressable bullet on it,
/// is refused rather than taken with it, and so is a bullet sharing
/// its line with the parent that opened it; the line itself goes
/// whole, whatever blocks its own text parses into, a spelled heading
/// or a quoted remark alike. Cutting an ordered item renumbers the
/// items after it downward, the way deleting from a numbered list does
/// everywhere; a cut that would raise a survivor's rendered number is
/// refused instead.
///
/// # Errors
///
/// Returns an error when a query is empty or spans lines, when the
/// scope or `query` matches no bullet or more than one, when the
/// matched bullet holds nested content or shares its line, when the
/// removal would change the structure around what it removes, or when
/// the note uses bare carriage-return line endings.
pub fn removed(text: &str, query: &str, scope: &Placement) -> Result<String, Error> {
    for heading in &scope.headings {
        validated(heading, Kind::Heading)?;
    }
    for bullet in &scope.bullets {
        validated(bullet, Kind::Bullet)?;
    }
    validated(query, Kind::Bullet)?;
    if bare_carriage_return(text) {
        return Err(Error::BareCarriageReturn);
    }
    let body_start = frontmatter::body_start(text);
    let body = &text[body_start..];
    let outline = outline(body);
    let range = edit_scope(body, &outline, scope)?;
    let index = one_bullet(body, &outline, &range, query)?;
    let bullet = &outline.bullets[index];
    let start = line_start(body, content_start(body, &bullet.span));
    let line_end = start + line_len(&body[start..]);
    // A child can open on the marker line itself, so nested bullets
    // are detected structurally, not by what follows the first line.
    let within = interior(body, bullet);
    if outline
        .bullets
        .iter()
        .enumerate()
        .any(|(other, child)| other != index && contains(&within, body, &child.span))
    {
        return Err(Error::NestedContent {
            query: query.to_owned(),
        });
    }
    if line_end < bullet.span.end && !blank(&body[line_end..bullet.span.end]) {
        return Err(Error::NestedContent {
            query: query.to_owned(),
        });
    }
    // A marker-line child shares its line with its parent; cutting the
    // line would take a bullet the query never named.
    if content_start(body, &(start..line_end)) != content_start(body, &bullet.span) {
        return Err(Error::SharedLine {
            query: query.to_owned(),
        });
    }
    // The cut takes exactly the named line, never a neighbor or a
    // blank separator, and must leave the surroundings meaning what
    // they meant; where it cannot, the removal is refused.
    let before = blocks(body);
    let candidate = excised(text, body_start + start..body_start + line_end);
    if removal_kept(&candidate, &before, body, body_start, &(start..line_end)) {
        return Ok(candidate);
    }
    Err(Error::RemovalReshapes)
}

/// `text` with the one task `query` names checked or unchecked: the
/// box's state character flipped in place, nothing else touched. A
/// task is a bullet whose text starts with `[ ]`, `[x]`, or `[X]`;
/// `query` is a prefix of the task's text past the box, so the same
/// query matches before and after checking, and only task bullets are
/// candidates. A task already in the asked state comes back unchanged.
/// The flip is a same-length edit of inline text, which cannot change
/// the parse, so no structural check applies.
///
/// # Errors
///
/// Returns an error when a query is empty or spans lines, when the
/// scope matches no heading or bullet or more than one, when `query`
/// matches no task or more than one, or when the note uses bare
/// carriage-return line endings.
pub fn toggled(text: &str, query: &str, scope: &Placement, checked: bool) -> Result<String, Error> {
    for heading in &scope.headings {
        validated(heading, Kind::Heading)?;
    }
    for bullet in &scope.bullets {
        validated(bullet, Kind::Bullet)?;
    }
    validated(query, Kind::Bullet)?;
    if bare_carriage_return(text) {
        return Err(Error::BareCarriageReturn);
    }
    let body_start = frontmatter::body_start(text);
    let body = &text[body_start..];
    let outline = outline(body);
    let range = edit_scope(body, &outline, scope)?;
    // A wide gap after the marker makes the rest of the line indented
    // code inside the item; a box the parser reads as code is content,
    // never a task.
    let code: Vec<Range<usize>> = blocks(body)
        .into_iter()
        .filter_map(|(kind, _, span)| matches!(kind, BlockKind::Code).then_some(span))
        .collect();
    let mut found = None;
    for bullet in &outline.bullets {
        if !contains(&range, body, &bullet.span) {
            continue;
        }
        let Some((task, state)) = task_text(bullet_line(body, bullet)) else {
            continue;
        };
        let at = content_start(body, &bullet.span) + state;
        if code.iter().any(|span| span.contains(&at)) {
            continue;
        }
        if !task.starts_with(query) {
            continue;
        }
        if found.is_some() {
            return Err(Error::AmbiguousTask {
                query: query.to_owned(),
            });
        }
        found = Some(body_start + at);
    }
    let Some(at) = found else {
        return Err(Error::NoTask {
            query: query.to_owned(),
        });
    };
    let current = text.as_bytes()[at];
    let done = current == b'x' || current == b'X';
    if done == checked {
        return Ok(text.to_owned());
    }
    let mut new = text.to_owned();
    new.replace_range(at..=at, if checked { "x" } else { " " });
    Ok(new)
}

/// The text of a task line past its box, with the byte offset of the
/// box's state character from the line's start: a task's text begins
/// `[ ]`, `[x]`, or `[X]` directly past the marker, followed by
/// whitespace or the line's end. The whitespace after the box is
/// separator, not text, so a query never has to spell it. Any other
/// line is no task.
fn task_text(line: &str) -> Option<(&str, usize)> {
    let text = bullet_text(line);
    let state = line.len() - text.len() + 1;
    let rest = text
        .strip_prefix('[')?
        .strip_prefix([' ', 'x', 'X'])?
        .strip_prefix(']')?;
    if rest.is_empty() {
        return Some((rest, state));
    }
    let task = rest.trim_start_matches([' ', '\t']);
    if task.len() == rest.len() {
        return None;
    }
    Some((task, state))
}

/// The note with `range` cut out.
fn excised(text: &str, range: Range<usize>) -> String {
    let mut new = String::with_capacity(text.len() - range.len());
    new.push_str(&text[..range.start]);
    new.push_str(&text[range.end..]);
    new
}

/// Checks an excised note against the blocks of the original body.
/// Block spans shed and absorb neighboring blank lines and indent as
/// whitespace ownership shifts across an edit, so blocks compare by
/// their content edges: a block whose content sits inside the removed
/// range vanishes with it, and every other block survives with kind,
/// depth, and its known content edges intact. A straddling container
/// whose content begins or ends on the removed line takes its new
/// edge from a surviving child, so that side goes unpinned; an
/// ordered list losing its first item renumbers by exactly one; code
/// and raw HTML render their whitespace, so their spans must survive
/// byte-exact. Nothing may appear that was not there, and every
/// remaining heading keeps the parent heading it had.
fn removal_kept(
    new: &str,
    before: &[(BlockKind, usize, Range<usize>)],
    body: &str,
    boundary: usize,
    removed: &Range<usize>,
) -> bool {
    if frontmatter::body_start(new) != boundary {
        return false;
    }
    // Compared offsets are content edges outside the cut; an offset
    // inside it clamps to the cut start, where at worst it fails a
    // match instead of wrapping.
    let length = removed.end - removed.start;
    let map = |offset: usize| {
        if offset <= removed.start {
            offset
        } else {
            offset.saturating_sub(length).max(removed.start)
        }
    };
    let new_body = &new[boundary..];
    let after = blocks(new_body);
    let mut cursor = 0;
    let mut matched = 0;
    'blocks: for (kind, depth, span) in before {
        let lead = content_start(body, span);
        let tail = content_end(body, span);
        if lead >= removed.start && tail <= removed.end {
            continue;
        }
        let lead_known = lead < removed.start || lead >= removed.end;
        let tail_known = tail <= removed.start || tail > removed.end;
        let lead = if lead_known { map(lead) } else { removed.start };
        let tail = map(tail);
        let literal = matches!(kind, BlockKind::Code | BlockKind::Html);
        let end = map(span.end);
        while cursor < after.len() {
            let (other, nested, candidate) = &after[cursor];
            let found = content_start(new_body, candidate);
            // A matching candidate never overshoots: a known lead is
            // matched exactly, and no lead reaches its block's end.
            if found > lead && (lead_known || found >= tail) {
                break;
            }
            if kind_kept(*kind, *other, !lead_known)
                && nested == depth
                && (!lead_known || found == lead)
                && (!tail_known || content_end(new_body, candidate) == tail)
                && (!literal || candidate.end == end)
            {
                cursor += 1;
                matched += 1;
                continue 'blocks;
            }
            cursor += 1;
        }
        return false;
    }
    // A removal only ever deletes blocks: an unmatched leftover means
    // the cut manufactured structure, like a blank line promoted to a
    // list separator, reshaping bullets the query never named.
    if matched != after.len() {
        return false;
    }
    let expected: Vec<(HeadingLevel, usize)> = before
        .iter()
        .filter_map(|(kind, _, span)| {
            let BlockKind::Heading(level) = kind else {
                return None;
            };
            let lead = content_start(body, span);
            if lead >= removed.start && content_end(body, span) <= removed.end {
                return None;
            }
            Some((*level, map(span.start)))
        })
        .collect();
    let found: Vec<(HeadingLevel, usize)> = after
        .iter()
        .filter_map(|(kind, _, span)| {
            let BlockKind::Heading(level) = kind else {
                return None;
            };
            Some((*level, span.start))
        })
        .collect();
    let wanted = parented(&expected);
    let real = parented(&found);
    wanted
        .iter()
        .all(|(start, parent)| real.iter().any(|pair| pair == &(*start, *parent)))
}

/// A heading in the body: its level and the byte range of its source,
/// the underline included for setext.
struct Heading {
    level: HeadingLevel,
    span: Range<usize>,
}

/// A list item: the byte range of its source, children included.
struct Bullet {
    span: Range<usize>,
}

/// A block an insertion must stay out of — fenced code or raw HTML —
/// with the content column of its container: the reference for closing
/// indents, and for whether an entry's own indent escapes the container
/// and ends the block at its boundary.
struct Contained {
    span: Range<usize>,
    container: usize,
}

/// Every heading and bullet that can be a target, in document order,
/// and the raw HTML blocks, fenced code, and block quotes an insertion
/// must stay out of.
struct Outline {
    headings: Vec<Heading>,
    bullets: Vec<Bullet>,
    html: Vec<Contained>,
    fenced: Vec<Contained>,
    quotes: Vec<Range<usize>>,
}

/// One walk over the body's events. Quoted structure is skipped because
/// an append cannot extend a quotation, and a heading inside a list item
/// is item content, not a section boundary: a column-zero heading always
/// terminates the list first, so outline headings can never sit inside
/// an item.
fn outline(body: &str) -> Outline {
    let mut headings = Vec::new();
    let mut bullets = Vec::new();
    let mut html = Vec::new();
    let mut fenced = Vec::new();
    let mut depth = 0_usize;
    let mut item_columns = Vec::new();
    let mut quotes = Vec::new();
    for (event, span) in Parser::new(body).into_offset_iter() {
        match event {
            Event::Start(Tag::BlockQuote(_)) => {
                if depth == 0 {
                    quotes.push(span);
                }
                depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => depth -= 1,
            Event::Start(Tag::Item) => {
                if depth == 0 {
                    bullets.push(Bullet { span: span.clone() });
                }
                item_columns.push(content_column(body, &span));
            }
            Event::End(TagEnd::Item) => {
                item_columns.pop();
            }
            Event::Start(Tag::Heading { level, .. }) if depth == 0 && item_columns.is_empty() => {
                headings.push(Heading { level, span });
            }
            Event::Start(Tag::HtmlBlock) if depth == 0 => {
                html.push(Contained {
                    span,
                    container: item_columns.last().copied().unwrap_or(0),
                });
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) if depth == 0 => {
                fenced.push(Contained {
                    span,
                    container: item_columns.last().copied().unwrap_or(0),
                });
            }
            _ => {}
        }
    }
    Outline {
        headings,
        bullets,
        html,
        fenced,
        quotes,
    }
}

/// Which kind of query failed validation, for the error message.
#[derive(Clone, Copy)]
enum Kind {
    Heading,
    Bullet,
}

/// A query names one line of the note, so it must be one non-empty line.
fn validated(query: &str, kind: Kind) -> Result<(), Error> {
    let reason = if query.is_empty() {
        "it is empty"
    } else if query.contains(['\n', '\r']) {
        "it contains a line break"
    } else {
        return Ok(());
    };
    let query = query.to_owned();
    Err(match kind {
        Kind::Heading => Error::InvalidHeading { query, reason },
        Kind::Bullet => Error::InvalidBullet { query, reason },
    })
}

/// The scope a placement narrows a placed edit to: the innermost
/// matched section, then the interior of the last scoped bullet when
/// the placement descends a thread.
fn edit_scope(body: &str, outline: &Outline, scope: &Placement) -> Result<Range<usize>, Error> {
    let section = heading_scope(body, outline, &scope.headings)?;
    if scope.bullets.is_empty() {
        return Ok(section);
    }
    let index = matched_bullet(body, outline, section, &scope.bullets)?;
    Ok(interior(body, &outline.bullets[index]))
}

/// The scope a heading path narrows to: the whole body for an empty
/// path, else the innermost matched section.
fn heading_scope(body: &str, outline: &Outline, path: &[String]) -> Result<Range<usize>, Error> {
    let mut scope = 0..body.len();
    for query in path {
        let index = matched_heading(body, outline, &scope, query)?;
        scope = section(body, outline, index);
    }
    Ok(scope)
}

/// The one heading inside `scope` whose text begins with `query`.
fn matched_heading(
    body: &str,
    outline: &Outline,
    scope: &Range<usize>,
    query: &str,
) -> Result<usize, Error> {
    let mut found = None;
    for (index, heading) in outline.headings.iter().enumerate() {
        if !contains(scope, body, &heading.span) || !heading_matches(body, heading, query) {
            continue;
        }
        if found.is_some() {
            return Err(Error::AmbiguousHeading {
                query: query.to_owned(),
            });
        }
        found = Some(index);
    }
    found.ok_or_else(|| Error::NoHeading {
        query: query.to_owned(),
    })
}

/// Whether the heading's text begins with `query`, byte-exact:
/// a target is quoted from the note, not searched for.
fn heading_matches(body: &str, heading: &Heading, query: &str) -> bool {
    let start = content_start(body, &heading.span);
    heading_text(first_line(&body[start..heading.span.end])).starts_with(query)
}

/// The section a heading opens: from past its source, which ends in a
/// line break everywhere but the end of the note, to the next heading of
/// the same or a shallower level, or the body's end. Any heading
/// strictly inside a section is deeper by construction, so path segments
/// need no explicit level check.
fn section(body: &str, outline: &Outline, index: usize) -> Range<usize> {
    let heading = &outline.headings[index];
    let end = outline.headings[index + 1..]
        .iter()
        .find(|next| next.level <= heading.level)
        .map_or(body.len(), |next| line_start(body, next.span.start));
    heading.span.end..end
}

/// The one bullet a bullet path names inside `scope`: the first segment
/// anywhere in the scope at any depth, each later segment strictly
/// inside the previous match's thread.
fn matched_bullet(
    body: &str,
    outline: &Outline,
    scope: Range<usize>,
    path: &[String],
) -> Result<usize, Error> {
    let mut index = one_bullet(body, outline, &scope, &path[0])?;
    for query in &path[1..] {
        let scope = interior(body, &outline.bullets[index]);
        index = one_bullet(body, outline, &scope, query)?;
    }
    Ok(index)
}

/// The one bullet inside `scope` whose text begins with `query`,
/// byte-exact like [`heading_matches`].
fn one_bullet(
    body: &str,
    outline: &Outline,
    scope: &Range<usize>,
    query: &str,
) -> Result<usize, Error> {
    let mut found = None;
    for (index, bullet) in outline.bullets.iter().enumerate() {
        if !contains(scope, body, &bullet.span)
            || !bullet_text(bullet_line(body, bullet)).starts_with(query)
        {
            continue;
        }
        if found.is_some() {
            return Err(Error::AmbiguousBullet {
                query: query.to_owned(),
            });
        }
        found = Some(index);
    }
    found.ok_or_else(|| Error::NoBullet {
        query: query.to_owned(),
    })
}

/// The descendant range of a bullet: past its marker and the gap after
/// it, through the end of its thread. A child can open on the marker
/// line itself (`- - p`), so only the marker is skipped, never the
/// line.
fn interior(body: &str, bullet: &Bullet) -> Range<usize> {
    let start = content_start(body, &bullet.span);
    let first = first_line(&body[start..bullet.span.end]);
    let text = start + (first.len() - bullet_text(first).len());
    text..bullet.span.end
}

/// Whether a span sits inside `scope`. The span's content start stands
/// in for its leading edge, which is not a line boundary: an indented
/// heading's span starts at its marker, and a tab-nested item's span
/// reaches back to the terminator before its indent.
fn contains(scope: &Range<usize>, body: &str, span: &Range<usize>) -> bool {
    scope.start <= content_start(body, span) && span.end <= scope.end
}

/// The offset just past a span's last non-whitespace byte, where its
/// content actually ends. Block spans absorb neighboring blank lines
/// and the next line's indent as whitespace ownership shifts across an
/// edit, so structural comparisons anchor on content, not span edges.
fn content_end(body: &str, span: &Range<usize>) -> usize {
    let trimmed = body[span.start..span.end].trim_end_matches([' ', '\t', '\r', '\n']);
    span.start + trimmed.len()
}

/// The offset of a span's first non-whitespace byte, where its marker or
/// text actually begins.
fn content_start(body: &str, span: &Range<usize>) -> usize {
    let offset = body[span.start..span.end]
        .find(|character: char| !matches!(character, ' ' | '\t' | '\r' | '\n'))
        .expect("a heading or item span always holds its marker");
    span.start + offset
}

/// A source slice's first line, its terminator excluded.
fn first_line(source: &str) -> &str {
    match source.find(['\n', '\r']) {
        Some(end) => &source[..end],
        None => source,
    }
}

/// The matchable text of a heading's first line: past an ATX marker run
/// and the whitespace separating it from the text, or a setext first
/// line as written. A `#` run not followed by whitespace, or longer
/// than the six ATX allows, is not a marker, so a setext heading's text
/// keeps its hashes.
fn heading_text(line: &str) -> &str {
    let rest = line.trim_start_matches('#');
    if rest.len() == line.len() || line.len() - rest.len() > 6 {
        return line;
    }
    if rest.is_empty() {
        return rest;
    }
    let text = rest.trim_start_matches([' ', '\t']);
    if text.len() == rest.len() {
        return line;
    }
    closing_stripped(text)
}

/// ATX text without its optional closing sequence: a trailing `#` run
/// preceded by whitespace, or standing alone, is syntax, so `# Foo #`
/// reads as `Foo` the way an editor shows it — while the run in `# C#`
/// touches the text and stays content.
fn closing_stripped(text: &str) -> &str {
    let trimmed = text.trim_end_matches([' ', '\t']);
    let stripped = trimmed.trim_end_matches('#');
    if stripped.len() == trimmed.len() {
        return trimmed;
    }
    if stripped.is_empty() {
        return stripped;
    }
    match stripped.strip_suffix([' ', '\t']) {
        Some(_) => stripped.trim_end_matches([' ', '\t']),
        None => trimmed,
    }
}

/// The first line of a bullet's own source: the line holding its marker,
/// reached from the span's content start because the span's leading edge
/// can sit before the marker's line.
fn bullet_line<'a>(body: &'a str, bullet: &Bullet) -> &'a str {
    let start = content_start(body, &bullet.span);
    first_line(&body[start..bullet.span.end])
}

/// The matchable text of a bullet's first line: past its marker and the
/// whitespace separating it from the text. A bare marker has no text and
/// matches no query, which is never a loss: it also has no words to be
/// named by.
fn bullet_text(line: &str) -> &str {
    let rest = line
        .strip_prefix(['-', '*', '+'])
        .unwrap_or_else(|| ordered_stripped(line));
    rest.trim_start_matches([' ', '\t'])
}

/// The line past an ordered marker: digits, then a dot or parenthesis.
/// Only called on item lines, which always carry a marker.
fn ordered_stripped(line: &str) -> &str {
    let rest = line.trim_start_matches(|character: char| character.is_ascii_digit());
    &rest[1..]
}

/// The start of the line holding `offset`. Lines end at a newline, a
/// bare carriage return, or the pair — the three endings the parser
/// recognizes — and every helper here shares that definition.
fn line_start(body: &str, offset: usize) -> usize {
    body[..offset].rfind(['\n', '\r']).map_or(0, |end| end + 1)
}

/// The length of a source slice's first line, its terminator included;
/// the whole slice when nothing ends it.
fn line_len(source: &str) -> usize {
    match source.find(['\n', '\r']) {
        Some(end) if source[end..].starts_with("\r\n") => end + 2,
        Some(end) => end + 1,
        None => source.len(),
    }
}

/// The insertion offset for a scope: just past its last non-blank line,
/// so trailing blank lines — the separation before whatever follows —
/// stay after the entry. An all-blank scope inserts at its start, which
/// for a section is directly past the heading.
fn insertion_at(body: &str, scope: &Range<usize>) -> usize {
    let mut at = scope.start;
    let mut offset = scope.start;
    while offset < scope.end {
        let length = line_len(&body[offset..scope.end]);
        if !blank(&body[offset..offset + length]) {
            at = offset + length;
        }
        offset += length;
    }
    at
}

/// Whether a line holds nothing but whitespace.
fn blank(line: &str) -> bool {
    line.bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
}

/// Refuses an insertion offset inside an unclosed fenced code block: the
/// entry would read as code, and only a closing fence — content, not
/// kladde's to write — could keep it out. A closed fence's span keeps
/// its closing line, so the offset can only sit past it: at the span's
/// end when the note cut the trailing newline off, past it otherwise.
/// An entry indented below the fence's container ends that container —
/// and the fence with it — so it needs no refusing.
fn outside_fences(body: &str, outline: &Outline, at: usize, entry: usize) -> Result<(), Error> {
    for fence in &outline.fenced {
        if fence.span.start >= at || at > fence.span.end || entry < fence.container {
            continue;
        }
        if !closed_fence(body, fence) {
            return Err(Error::UnclosedFence);
        }
    }
    Ok(())
}

/// Whether a fenced code block's source ends with a valid closing fence:
/// a line of at least as many of the opening's fence characters and
/// nothing else, indented no more than three columns past the
/// container's content column — a deeper fence line is code, as the
/// parser read it. The opening line can never close itself.
fn closed_fence(body: &str, fence: &Contained) -> bool {
    let source = &body[fence.span.start..fence.span.end];
    let Some(last) = source.trim_end_matches(['\r', '\n']).rfind(['\n', '\r']) else {
        return false;
    };
    let opening = first_line(source);
    let character = opening.as_bytes()[0];
    let run = opening.len() - opening.trim_start_matches(character as char).len();
    let line = first_line(&source[last + 1..]);
    let closing = line.trim_start_matches([' ', '\t']);
    let indent = columns(&line[..line.len() - closing.len()]);
    // Only trailing spaces are allowed after a closing fence; a tab
    // keeps the line inside the block, as the parser reads it.
    let closing = closing.trim_end_matches(' ');
    indent <= fence.container + 3
        && closing.len() >= run
        && closing.bytes().all(|byte| byte == character)
}

/// The insertion offset moved out of raw HTML and block quotes, with
/// whether a blank line must precede the entry. An offset at or before
/// such a block's end would smuggle the entry in — into HTML as more of
/// the block, into a quotation as a lazy continuation — so for a
/// blank-terminated block it advances past the terminating blank (later
/// blanks are separation and stay after the entry), and when a
/// container or the end of the note cut the block off before any blank,
/// one is written instead. A marker-terminated HTML block that closed
/// itself needs neither, and an unclosed one is refused: no blank can
/// end it — though an entry indented below the block's container ends
/// the container, and the HTML with it, and needs nothing. Quotations
/// get no such pass: lazy continuation reaches across a container's
/// end.
fn past_blocks(
    body: &str,
    outline: &Outline,
    at: usize,
    entry: usize,
) -> Result<(usize, bool), Error> {
    for block in &outline.html {
        if block.span.start >= at || at > block.span.end || entry < block.container {
            continue;
        }
        let source = &body[block.span.start..block.span.end];
        return match html_end_markers(first_line(source)) {
            Some(markers) if !closed_html(source, markers) => Err(Error::UnclosedHtml),
            Some(_) => Ok((at, false)),
            None => Ok(past_terminator(body, block.span.end, at)),
        };
    }
    for span in &outline.quotes {
        if span.start < at && at <= span.end {
            return Ok(past_terminator(body, span.end, at));
        }
    }
    Ok((at, false))
}

/// Where an entry lands after a blank-terminated block ending at `end`:
/// past the terminating blank when the note holds one, else at `at`,
/// with a blank line to be written before the entry.
fn past_terminator(body: &str, end: usize, at: usize) -> (usize, bool) {
    let length = line_len(&body[end..]);
    if length > 0 && blank(&body[end..end + length]) {
        (end + length, false)
    } else {
        (at, true)
    }
}

/// Whether the note holds a carriage return with no newline after it.
/// The parser reads such line endings inconsistently — fence scanning
/// runs through them while other blocks break at them — so no placement
/// into the note can be trusted, and it is refused. The end-of-note
/// append stays available: it claims no structure.
fn bare_carriage_return(text: &str) -> bool {
    let mut rest = text;
    while let Some(index) = rest.find('\r') {
        if !rest[index..].starts_with("\r\n") {
            return true;
        }
        rest = &rest[index + 2..];
    }
    false
}

/// The end markers that can close a raw HTML block, from its opening
/// line: the marker-terminated kinds are raw-text elements, comments,
/// processing instructions, declarations, and CDATA. `None` is every
/// other block, which only a blank line (or the end of its container)
/// ends. A raw-text element closes only at its own closing tag — the
/// parser keeps `<script>` open across a stray `</style>`, measured.
fn html_end_markers(opening: &str) -> Option<&'static [&'static str]> {
    let lower = opening.to_ascii_lowercase();
    let raw_text: [(&str, &'static [&'static str]); 4] = [
        ("<script", &["</script>"]),
        ("<pre", &["</pre>"]),
        ("<style", &["</style>"]),
        ("<textarea", &["</textarea>"]),
    ];
    for (tag, markers) in raw_text {
        if let Some(rest) = lower.strip_prefix(tag)
            && matches!(
                rest.bytes().next(),
                None | Some(b' ' | b'\t' | b'>' | b'\r' | 0x0B | 0x0C)
            )
        {
            return Some(markers);
        }
    }
    if opening.starts_with("<!--") {
        Some(&["-->"])
    } else if opening.starts_with("<?") {
        Some(&["?>"])
    } else if opening.starts_with("<![CDATA[") {
        Some(&["]]>"])
    } else if opening.starts_with("<!")
        && opening[2..].starts_with(|character: char| character.is_ascii_alphabetic())
    {
        Some(&[">"])
    } else {
        None
    }
}

/// Whether a marker-terminated HTML block's last line holds one of its
/// end markers, compared case-insensitively the way the markers close.
/// The last line is found past one stripped terminator, never by byte
/// arithmetic: the source can end mid-character.
fn closed_html(source: &str, markers: &[&str]) -> bool {
    let content = source.strip_suffix('\n').unwrap_or(source);
    let content = content.strip_suffix('\r').unwrap_or(content);
    let last = source[line_start(content, content.len())..].to_ascii_lowercase();
    markers.iter().any(|marker| last.contains(marker))
}

/// The indent for an entry nested under the bullet at `index`: the last
/// DIRECT child's indent when children exist — the entry lands at the
/// end of the thread, where that child is its sibling, and the first
/// child's indent could nest the entry under a differently-indented
/// last one — else enough of the configured unit to reach the bullet's
/// content column, where `CommonMark` nests an item. Indents are always
/// whitespace: a child that opens on its parent's marker line has a
/// marker where its indent would be, and gets column-equivalent spaces
/// instead.
fn entry_indent(body: &str, outline: &Outline, index: usize, indent: Indent) -> String {
    let bullet = &outline.bullets[index];
    let within = interior(body, bullet);
    let mut sibling: Option<&Bullet> = None;
    for child in &outline.bullets[index + 1..] {
        if !contains(&within, body, &child.span) {
            continue;
        }
        let descendant =
            sibling.is_some_and(|prev| contains(&interior(body, prev), body, &child.span));
        if !descendant {
            sibling = Some(child);
        }
    }
    if let Some(child) = sibling {
        let start = content_start(body, &child.span);
        let prefix = &body[line_start(body, start)..start];
        return if prefix.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
            prefix.to_owned()
        } else {
            " ".repeat(columns(prefix))
        };
    }
    let start = content_start(body, &bullet.span);
    let own = &body[line_start(body, start)..start];
    let leading = &own[..own.len() - own.trim_start_matches([' ', '\t']).len()];
    let content = content_column(body, &bullet.span);
    match indent {
        Indent::Tab => {
            // The first tab stop at or past the content column is short
            // of the indented-code cutoff four columns later, so the
            // entry always parses as a child.
            let mut prefix = leading.to_owned();
            let mut reached = columns(leading);
            loop {
                prefix.push('\t');
                reached += 4 - reached % 4;
                if reached >= content {
                    return prefix;
                }
            }
        }
        Indent::Spaces => " ".repeat(content),
    }
}

/// The column where an item's own text starts: where `CommonMark` nests
/// its children and measures its inner indentation from. More than four
/// columns of padding reads as one space of padding plus indented code,
/// and a bare marker never wrote its one space, so both snap back to
/// the marker.
fn content_column(body: &str, span: &Range<usize>) -> usize {
    let start = content_start(body, span);
    let line = line_start(body, start);
    let first = first_line(&body[start..span.end]);
    let after = first
        .strip_prefix(['-', '*', '+'])
        .unwrap_or_else(|| ordered_stripped(first));
    let marker = columns(&body[line..start + (first.len() - after.len())]);
    let text = bullet_text(first);
    if text.is_empty() {
        return marker + 1;
    }
    let text = columns(&body[line..start + (first.len() - text.len())]);
    if text - marker > 4 { marker + 1 } else { text }
}

/// The display column reached after `text`, a tab advancing to the next
/// four-column stop, the width `CommonMark` measures indentation with.
fn columns(text: &str) -> usize {
    let mut column = 0;
    for character in text.chars() {
        column += if character == '\t' { 4 - column % 4 } else { 1 };
    }
    column
}

/// The terminator for inserted lines: the last one appearing before the
/// insertion point, LF in a note that has none yet. Callers pass the
/// full note prefix, frontmatter included, so a CRLF block's style
/// carries into a body that has no terminator of its own to show.
fn insertion_eol(before: &str) -> &'static str {
    match before.rfind('\n') {
        Some(newline) if before[..newline].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

/// The entry as spliceable lines: each prefixed with `indent` and closed
/// with `eol`, terminators inside the entry — bare carriage returns
/// included — normalized to `eol`. A blank entry line stays bare, so no
/// line gains trailing whitespace.
fn rendered(entry: &str, indent: &str, eol: &str) -> String {
    let mut lines = String::new();
    let mut rest = entry;
    loop {
        let length = line_len(rest);
        let line = rest[..length].trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            lines.push_str(eol);
        } else {
            lines.push_str(indent);
            lines.push_str(line);
            lines.push_str(eol);
        }
        rest = &rest[length..];
        if rest.is_empty() {
            return lines;
        }
    }
}

/// `text` with `lines` inserted at `at`, preceded by a terminator when
/// the text before the insertion point does not end in one, which only
/// happens at the unterminated end of a note: bare carriage returns
/// never get this far.
fn spliced(text: &str, at: usize, lines: &str, eol: &str) -> String {
    let mut new = String::with_capacity(text.len() + lines.len() + eol.len());
    new.push_str(&text[..at]);
    if !new.ends_with('\n') {
        new.push_str(eol);
    }
    new.push_str(lines);
    new.push_str(&text[at..]);
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(headings: &[&str], bullets: &[&str]) -> Placement {
        Placement {
            headings: headings.iter().map(|&query| query.to_owned()).collect(),
            bullets: bullets.iter().map(|&query| query.to_owned()).collect(),
            indent: Indent::Tab,
        }
    }

    fn spaced(headings: &[&str], bullets: &[&str]) -> Placement {
        Placement {
            indent: Indent::Spaces,
            ..placed(headings, bullets)
        }
    }

    /// The parser reads bare carriage returns inconsistently — fence
    /// scanning runs through them while other blocks break at them — so
    /// no placement into such a note can be trusted, and every one is
    /// refused. An entry's own bare returns are still normalized: they
    /// never reach the note.
    #[test]
    fn inserted_refuses_bare_carriage_returns() {
        let cases = [
            ("# A\rbody\r", placed(&["A"], &[])),
            ("- p\r  - c\r", placed(&[], &["p"])),
            ("# A\r\nbody\r", placed(&["A"], &[])),
            ("# A\n```\r#\r`", placed(&["A"], &[])),
        ];
        for (text, placement) in cases {
            let error = inserted(text, "- e", &placement).expect_err("bare CR refuses");
            assert_eq!(
                error.to_string(),
                "the note uses bare carriage-return line endings",
                "for {text:?}"
            );
        }
        let entry = inserted("- a\n", "- e\rmore", &placed(&[], &["a"])).expect("insert succeeds");
        assert_eq!(entry, "- a\n\t- e\n\tmore\n");
    }

    /// The one fence in `text`'s outline, as its span and container
    /// content column.
    fn fence_of(text: &str) -> (Range<usize>, usize) {
        let outline = outline(text);
        assert_eq!(outline.fenced.len(), 1, "for {text:?}");
        let fence = &outline.fenced[0];
        (fence.span.clone(), fence.container)
    }

    /// See [`outline_pins_heading_spans`]; a CLOSED fence's span drops
    /// the newline after its closing line, so an insertion offset can
    /// only reach a closed span's end when the note cut that newline
    /// off — every other in-span offset means the fence is open. An
    /// over-indented fence line is code, not a closer, and the block
    /// runs on.
    #[test]
    fn outline_pins_fenced_spans() {
        assert_eq!(fence_of("```\ncode\n```\npara\n"), (0..12, 0));
        assert_eq!(fence_of("~~~\ncode\n"), (0..9, 0));
        assert_eq!(fence_of("```\n"), (0..4, 0));
        assert_eq!(fence_of("```rust\ncode\n```"), (0..16, 0));
        assert_eq!(fence_of("- p\n  ```\n  code\n"), (6..17, 2));
        assert_eq!(fence_of("~~~\ncode\n    ~~~\npara\n"), (0..22, 0));
        assert_eq!(fence_of("-\n  ```\n  code\n"), (4..15, 2));
    }

    #[test]
    fn closed_fence_reads_closing_lines() {
        let cases = [
            ("```\ncode\n```", 0, true),
            ("```rust\ncode\n  ```  ", 0, true),
            ("~~~\ncode\n~~~~", 0, true),
            ("```", 0, false),
            ("```\ncode", 0, false),
            ("````\ncode\n```", 0, false),
            ("~~~\ncode\n```", 0, false),
            ("~~~\ncode\n    ~~~", 0, false),
            ("```\ncode\n     ```", 2, true),
            ("```\ncode\n     ```", 1, false),
            ("```\ncode\n``` ", 0, true),
            ("```\ncode\n```\t", 0, false),
        ];
        for (source, container, closed) in cases {
            let fence = Contained {
                span: 0..source.len(),
                container,
            };
            assert_eq!(closed_fence(source, &fence), closed, "for {source:?}");
        }
    }

    /// See [`outline_pins_heading_spans`]; a quote's span excludes its
    /// terminating blank like raw HTML's, and lazy continuation lines
    /// belong to the quote.
    #[test]
    fn outline_pins_quote_spans() {
        assert_eq!(outline("> a\n\npara\n").quotes, vec![0..4]);
        assert_eq!(outline("# A\n> quoted\nplain\n# B\n").quotes, vec![4..19]);
        assert_eq!(outline("- p\n  > q\n- q2\n").quotes, vec![6..10]);
        assert_eq!(outline("> > a\n").quotes, vec![0..6]);
    }

    /// A plain entry directly after a quotation would read as its lazy
    /// continuation, so the quote is terminated first: the entry lands
    /// past an existing terminating blank, or a blank is written.
    #[test]
    fn inserted_stays_out_of_quotations() {
        let text = "# A\n> quoted\n# B\n";
        let new = inserted(text, "plain", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n> quoted\n\nplain\n# B\n");
        let blanked = "# A\n> quoted\n\n# B\n";
        let new = inserted(blanked, "plain", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n> quoted\n\nplain\n# B\n");
        let lazy = "# A\n> quoted\nplain\n# B\n";
        let new = inserted(lazy, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n> quoted\nplain\n\n- e\n# B\n");
        let item = "- p\n  > q\n- q2\n";
        let new = inserted(item, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  > q\n\n\t- e\n- q2\n");
        let cut = "# A\n> q";
        let new = inserted(cut, "plain", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n> q\n\nplain\n");
        let behind = "# A\n> early\n\nlate\n# B\n";
        let new = inserted(behind, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n> early\n\nlate\n- e\n# B\n");
    }

    /// The one HTML span in `text`'s outline, with its container column.
    fn html_span(text: &str) -> (Range<usize>, usize) {
        let outline = outline(text);
        assert_eq!(outline.html.len(), 1, "for {text:?}");
        let block = &outline.html[0];
        (block.span.clone(), block.container)
    }

    /// See [`outline_pins_heading_spans`]; a raw HTML block's span
    /// EXCLUDES the blank line that terminates it (even a padded one),
    /// and a block cut off by the end of the note runs to the end.
    #[test]
    fn outline_pins_html_spans() {
        assert_eq!(html_span("<div>\nraw\n\npara\n"), (0..10, 0));
        assert_eq!(html_span("<div>\nraw\n \t\npara\n"), (0..10, 0));
        assert_eq!(html_span("<div>\nraw"), (0..9, 0));
        assert_eq!(html_span("- p\n  <div>\n  raw\n\n- q\n"), (6..18, 2));
        assert_eq!(html_span("<script>\na\n</script>\npara\n"), (0..21, 0));
    }

    #[test]
    fn indent_spells_both_units() {
        assert_eq!(Indent::Tab.as_str(), "tab");
        assert_eq!(Indent::Spaces.as_str(), "spaces");
    }

    #[test]
    fn indent_defaults_to_tab() {
        assert_eq!(Indent::default(), Indent::Tab);
    }

    /// The empty placement is the plain append: verbatim entry, very end
    /// of the note, one terminating newline, a separating one only when
    /// the note lacks it — the bytes [`crate::write`] always wrote.
    #[test]
    fn inserted_with_an_empty_placement_appends_at_the_end() {
        let cases = [
            ("", "- e", "- e\n"),
            ("x\n", "- e", "x\n- e\n"),
            ("x", "- e", "x\n- e\n"),
            ("x\n\n\n", "- e", "x\n\n\n- e\n"),
            ("# A\r\nx\r\n", "- e", "# A\r\nx\r\n- e\n"),
            ("x\n", "- e\nmore", "x\n- e\nmore\n"),
        ];
        for (text, entry, expected) in cases {
            let new = inserted(text, entry, &Placement::default()).expect("append succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    /// Pins the span shapes pulldown-cmark reports, the measured truth
    /// the module is built on: block spans include their terminators, an
    /// indented heading's span starts at its marker rather than its
    /// line, and a tab-nested item's span reaches back to the terminator
    /// before its indent. An upgrade that moves any edge surfaces here.
    #[test]
    fn outline_pins_heading_spans() {
        let atx = outline("# A\nbody\n");
        assert_eq!(atx.headings[0].span, 0..4);
        assert_eq!(atx.headings[0].level, HeadingLevel::H1);
        let indented = outline("  ## A\nbody\n");
        assert_eq!(indented.headings[0].span, 2..7);
        assert_eq!(indented.headings[0].level, HeadingLevel::H2);
        let setext = outline("Title\n=====\nbody\n");
        assert_eq!(setext.headings[0].span, 0..12);
        assert_eq!(setext.headings[0].level, HeadingLevel::H1);
        let dashes = outline("a\nb\n---\nbody\n");
        assert_eq!(dashes.headings[0].span, 0..8);
        assert_eq!(dashes.headings[0].level, HeadingLevel::H2);
        let unterminated = outline("# A");
        assert_eq!(unterminated.headings[0].span, 0..3);
        let crlf = outline("# A\r\nbody\r\n");
        assert_eq!(crlf.headings[0].span, 0..5);
    }

    /// See [`outline_pins_heading_spans`]; the loose item's span keeps
    /// its trailing blank line, which insertion must not land after.
    #[test]
    fn outline_pins_item_spans() {
        let nested = outline("- a\n  - b\n- c\n");
        let spans: Vec<_> = nested
            .bullets
            .iter()
            .map(|bullet| bullet.span.clone())
            .collect();
        assert_eq!(spans, [0..10, 6..10, 10..14]);
        let tabbed = outline("- a\n\t- b\n");
        let spans: Vec<_> = tabbed
            .bullets
            .iter()
            .map(|bullet| bullet.span.clone())
            .collect();
        assert_eq!(spans, [0..9, 3..9]);
        let loose = outline("- a\n\n- b\n\npara\n");
        let spans: Vec<_> = loose
            .bullets
            .iter()
            .map(|bullet| bullet.span.clone())
            .collect();
        assert_eq!(spans, [0..5, 5..10]);
        let ordered = outline("1. a\n1) b\n");
        let spans: Vec<_> = ordered
            .bullets
            .iter()
            .map(|bullet| bullet.span.clone())
            .collect();
        assert_eq!(spans, [0..5, 5..10]);
        let unterminated = outline("- a");
        assert_eq!(unterminated.bullets[0].span, 0..3);
    }

    /// Fenced and indented code and quoted structure never reach the
    /// outline, and a heading indented into a list item is item content.
    #[test]
    fn outline_skips_disguised_structure() {
        let cases = [
            ("```\n# fake\n- fake\n```\n", 0, 0),
            ("para\n\n    # fake\n", 0, 0),
            ("> # H\n> - x\n", 0, 0),
            ("- a\n  # inner\n- c\n", 0, 2),
        ];
        for (text, headings, bullets) in cases {
            let outline = outline(text);
            assert_eq!(outline.headings.len(), headings, "for {text:?}");
            assert_eq!(outline.bullets.len(), bullets, "for {text:?}");
        }
    }

    #[test]
    fn inserted_lands_at_a_section_end() {
        let text = "# A\nalpha\n# B\nbeta\n";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\nalpha\n- e\n# B\nbeta\n");
        let new = inserted(text, "- e", &placed(&["B"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\nalpha\n# B\nbeta\n- e\n");
    }

    #[test]
    fn inserted_matches_a_prefix_case_sensitively() {
        let text = "# Decisions\nx\n";
        let new = inserted(text, "- e", &placed(&["Dec"], &[])).expect("insert succeeds");
        assert_eq!(new, "# Decisions\nx\n- e\n");
        let error = inserted(text, "- e", &placed(&["dec"], &[])).expect_err("miss fails");
        assert_eq!(error.to_string(), "no heading matching \"dec\"");
    }

    #[test]
    fn inserted_descends_nested_headings() {
        let text = "# A\n## kladde\nx\n## other\ny\n# B\n## kladde\nz\n";
        let new = inserted(text, "- e", &placed(&["A", "kladde"], &[])).expect("insert succeeds");
        assert_eq!(
            new,
            "# A\n## kladde\nx\n- e\n## other\ny\n# B\n## kladde\nz\n"
        );
        let skipping = "# A\n### Deep\nx\n# B\n";
        let new = inserted(skipping, "- e", &placed(&["A", "Deep"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n### Deep\nx\n- e\n# B\n");
    }

    #[test]
    fn inserted_disambiguates_a_duplicate_by_path() {
        let text = "# A\n## Notes\nx\n# B\n## Notes\ny\n";
        let error = inserted(text, "- e", &placed(&["Notes"], &[])).expect_err("ambiguous fails");
        assert_eq!(error.to_string(), "multiple headings matching \"Notes\"");
        let new = inserted(text, "- e", &placed(&["B", "Notes"], &[])).expect("path succeeds");
        assert_eq!(new, "# A\n## Notes\nx\n# B\n## Notes\ny\n- e\n");
    }

    #[test]
    fn inserted_reports_an_ambiguous_prefix() {
        let text = "# Alpha\n# Alp\n";
        let error = inserted(text, "- e", &placed(&["Alp"], &[])).expect_err("ambiguous fails");
        assert_eq!(error.to_string(), "multiple headings matching \"Alp\"");
    }

    #[test]
    fn inserted_reads_every_heading_shape() {
        let setext = "Title\n=====\nx\n";
        let new = inserted(setext, "- e", &placed(&["Tit"], &[])).expect("insert succeeds");
        assert_eq!(new, "Title\n=====\nx\n- e\n");
        let closing = "## Foo ##\nx\n";
        let new = inserted(closing, "- e", &placed(&["Foo"], &[])).expect("insert succeeds");
        assert_eq!(new, "## Foo ##\nx\n- e\n");
        let hashes = "#hash\n===\nx\n";
        let new = inserted(hashes, "- e", &placed(&["#hash"], &[])).expect("insert succeeds");
        assert_eq!(new, "#hash\n===\nx\n- e\n");
        let seven = "####### Foo\n===\nx\n";
        let new = inserted(seven, "- e", &placed(&["####### Foo"], &[])).expect("insert succeeds");
        assert_eq!(new, "####### Foo\n===\nx\n- e\n");
        let error = inserted(seven, "- e", &placed(&["Foo"], &[])).expect_err("hashes are text");
        assert_eq!(error.to_string(), "no heading matching \"Foo\"");
        let indented = "  # A\nx\n# B\n";
        let new = inserted(indented, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "  # A\nx\n- e\n# B\n");
        let empty = "#\nx\n";
        let error = inserted(empty, "- e", &placed(&["x"], &[])).expect_err("no text to match");
        assert_eq!(error.to_string(), "no heading matching \"x\"");
    }

    #[test]
    fn inserted_keeps_trailing_blanks_after_the_entry() {
        let text = "# A\nx\n\n\n# B\n";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\nx\n- e\n\n\n# B\n");
    }

    #[test]
    fn inserted_fills_an_empty_section() {
        let empty = "# A\n# B\n";
        let new = inserted(empty, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n- e\n# B\n");
        let blank = "# A\n\n# B\n";
        let new = inserted(blank, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n- e\n\n# B\n");
    }

    #[test]
    fn inserted_appends_at_an_unterminated_end() {
        let body = "# A\nbody";
        let new = inserted(body, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\nbody\n- e\n");
        let bare = "# A";
        let new = inserted(bare, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n- e\n");
    }

    #[test]
    fn inserted_treats_code_as_content_not_structure() {
        let text = "# A\n```\n# fake\n- fake\n```\n# B\n";
        let error = inserted(text, "- e", &placed(&["fake"], &[])).expect_err("fence hides it");
        assert_eq!(error.to_string(), "no heading matching \"fake\"");
        let error = inserted(text, "- e", &placed(&[], &["fake"])).expect_err("fence hides it");
        assert_eq!(error.to_string(), "no bullet matching \"fake\"");
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n```\n# fake\n- fake\n```\n- e\n# B\n");
    }

    #[test]
    fn inserted_validates_queries() {
        let cases = [
            (placed(&[""], &[]), "invalid heading \"\": it is empty"),
            (placed(&[], &[""]), "invalid bullet \"\": it is empty"),
            (
                placed(&["a\nb"], &[]),
                "invalid heading \"a\\nb\": it contains a line break",
            ),
            (
                placed(&[], &["a\rb"]),
                "invalid bullet \"a\\rb\": it contains a line break",
            ),
        ];
        for (placement, message) in cases {
            let error = inserted("# A\n", "- e", &placement).expect_err("invalid query fails");
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn inserted_nests_under_a_childless_bullet_with_a_tab() {
        let text = "- parent\n- other\n";
        let new = inserted(text, "- e", &placed(&[], &["parent"])).expect("insert succeeds");
        assert_eq!(new, "- parent\n\t- e\n- other\n");
    }

    #[test]
    fn inserted_copies_an_existing_child_indent() {
        let text = "- parent\n  - child\n";
        let new = inserted(text, "- e", &placed(&[], &["parent"])).expect("insert succeeds");
        assert_eq!(new, "- parent\n  - child\n  - e\n");
        let tabbed = "- parent\n\t- child\n";
        let new = inserted(tabbed, "- e", &spaced(&[], &["parent"])).expect("insert succeeds");
        assert_eq!(new, "- parent\n\t- child\n\t- e\n");
    }

    #[test]
    fn inserted_nests_with_spaces_to_the_content_column() {
        let cases = [
            ("- p\n", "p", "- p\n  - e\n"),
            ("1. p\n", "p", "1. p\n   - e\n"),
            ("10. p\n", "p", "10. p\n    - e\n"),
            ("  - p\n", "p", "  - p\n    - e\n"),
            ("- a\n\t- p\n", "p", "- a\n\t- p\n      - e\n"),
            ("-     p\n", "p", "-     p\n  - e\n"),
        ];
        for (text, query, expected) in cases {
            let new = inserted(text, "- e", &spaced(&[], &[query])).expect("insert succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    /// One tab is not always enough: the child must reach the parent's
    /// content column, measured in tab-stop columns, or it silently
    /// fails to parse as a child at all.
    #[test]
    fn inserted_reaches_a_wide_content_column_with_tabs() {
        let cases = [
            ("100. p\n", "100. p\n\t\t- e\n"),
            ("-    p\n", "-    p\n\t\t- e\n"),
            ("  - p\n", "  - p\n  \t- e\n"),
            ("-     p\n", "-     p\n\t- e\n"),
        ];
        for (text, expected) in cases {
            let new = inserted(text, "- e", &placed(&[], &["p"])).expect("insert succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    #[test]
    fn inserted_lands_past_a_raw_html_terminator() {
        let text = "# A\n<div>\nraw\n\n# B\n";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<div>\nraw\n\n- e\n# B\n");
        let item = "- p\n  <div>\n  raw\n\n- q\n";
        let new = inserted(item, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  <div>\n  raw\n\n\t- e\n- q\n");
        let passed = "# A\n<div>\nraw\n\npara\n# B\n";
        let new = inserted(passed, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<div>\nraw\n\npara\n- e\n# B\n");
        // A tag that merely starts like a raw-text element is not one:
        // it terminates at a blank like any other tag.
        let pretend = "# A\n<pretend>\nraw\n\n# B\n";
        let new = inserted(pretend, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<pretend>\nraw\n\n- e\n# B\n");
    }

    /// A block cut off by the end of the note has no terminating blank,
    /// so one is written: without it the entry would continue the HTML.
    #[test]
    fn inserted_terminates_html_cut_by_the_notes_end() {
        let text = "# A\n<div>\nraw\n";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<div>\nraw\n\n- e\n");
        let bare = "# A\n<div>\nraw";
        let new = inserted(bare, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<div>\nraw\n\n- e\n");
    }

    /// HTML that closed itself with a marker line is already over; the
    /// entry lands right after it, and no blank is invented.
    #[test]
    fn inserted_leaves_self_closed_html_alone() {
        let text = "# A\n<script>\na\n</script>\n# B\n";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<script>\na\n</script>\n- e\n# B\n");
        let declaration = "# A\n<!DOCTYPE html>\n# B\n";
        let new = inserted(declaration, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<!DOCTYPE html>\n- e\n# B\n");
        // A blank after a marker-closed block is separation, not a
        // terminator, and stays after the entry like any trailing blank.
        let separated = "# A\n<script>\na\n</script>\n\n# B\n";
        let new = inserted(separated, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n<script>\na\n</script>\n- e\n\n# B\n");
        let thread = "- p\n  <script>\n  a\n  </script>\n\n- q\n";
        let new = inserted(thread, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  <script>\n  a\n  </script>\n\t- e\n\n- q\n");
    }

    /// A blank-terminated block cut off by its container before any
    /// blank gets the blank written, closing the HTML before the entry.
    #[test]
    fn inserted_closes_html_cut_by_a_sibling() {
        let text = "- p\n  <div>\n  raw\n- q\n";
        let new = inserted(text, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  <div>\n  raw\n\n\t- e\n- q\n");
    }

    /// Marker-terminated HTML that never reached its marker cannot be
    /// closed by whitespace, so the placement is refused.
    #[test]
    fn inserted_rejects_unclosed_raw_html() {
        let cases = [
            ("# A\n<script>\nraw\n", placed(&["A"], &[])),
            ("# A\n<script>\nraw\n</style>\npara\n", placed(&["A"], &[])),
            ("# A\n<script\u{c}>\nraw\n", placed(&["A"], &[])),
            ("# A\n<script>\né", placed(&["A"], &[])),
            ("# A\n<!-- note\n", placed(&["A"], &[])),
            ("# A\n<?process\n", placed(&["A"], &[])),
            ("# A\n<![CDATA[\n", placed(&["A"], &[])),
            ("- p\n  <script>\n  raw\n- q\n", placed(&[], &["p"])),
        ];
        for (text, placement) in cases {
            let error = inserted(text, "- e", &placement).expect_err("open HTML rejects");
            assert_eq!(
                error.to_string(),
                "the target ends inside unclosed raw HTML",
                "for {text:?}"
            );
        }
    }

    /// An entry inside an unclosed fence would read as code, and only a
    /// closing fence — content, not kladde's to write — could stop it.
    /// An over-indented fence line never closed anything.
    #[test]
    fn inserted_rejects_an_unclosed_fence() {
        let cases = [
            ("# A\n~~~\ncode\n", placed(&["A"], &[])),
            ("# A\n```\n", placed(&["A"], &[])),
            ("# A\n````\ncode\n```\n", placed(&["A"], &[])),
            ("# A\n~~~\ncode\n    ~~~\n", placed(&["A"], &[])),
            ("# A\n```\ncode\n```\t\n", placed(&["A"], &[])),
            ("- p\n  ```\n  code\n", placed(&[], &["p"])),
            ("- p\n  ```\n  code\n      ```\n", placed(&[], &["p"])),
        ];
        for (text, placement) in cases {
            let error = inserted(text, "- e", &placement).expect_err("open fence rejects");
            assert_eq!(
                error.to_string(),
                "the target ends inside an unclosed code fence",
                "for {text:?}"
            );
        }
    }

    /// A closed fence whose trailing newline the note cut off is the one
    /// closed fence an insertion offset can reach; the entry follows it,
    /// inside a list item too, where the closing indent is measured
    /// against the item's content column.
    #[test]
    fn inserted_appends_after_a_closed_fence_cut_by_the_notes_end() {
        let text = "# A\n```rust\ncode\n```";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n```rust\ncode\n```\n- e\n");
        let nested = "- p\n  ```\n  code\n  ```";
        let new = inserted(nested, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  ```\n  code\n  ```\n\t- e\n");
    }

    #[test]
    fn inserted_strips_a_closing_hash_run() {
        let text = "# Foo #\nx\n# Foo # bar\ny\n";
        let new = inserted(text, "- e", &placed(&["Foo #"], &[])).expect("insert succeeds");
        assert_eq!(new, "# Foo #\nx\n# Foo # bar\ny\n- e\n");
        let glued = "# C#\nx\n";
        let new = inserted(glued, "- e", &placed(&["C#"], &[])).expect("insert succeeds");
        assert_eq!(new, "# C#\nx\n- e\n");
        let hashes = "# #\nx\n";
        let error = inserted(hashes, "- e", &placed(&["#"], &[])).expect_err("no text to match");
        assert_eq!(error.to_string(), "no heading matching \"#\"");
    }

    #[test]
    fn inserted_descends_a_bullet_thread() {
        let text = "- a\n\t- b\n\t\t- c\n- d\n";
        let new = inserted(text, "- e", &placed(&[], &["a", "b"])).expect("insert succeeds");
        assert_eq!(new, "- a\n\t- b\n\t\t- c\n\t\t- e\n- d\n");
    }

    #[test]
    fn inserted_matches_a_deep_bullet_from_the_top() {
        let text = "- a\n\t- unique\n";
        let new = inserted(text, "- e", &placed(&[], &["unique"])).expect("insert succeeds");
        assert_eq!(new, "- a\n\t- unique\n\t\t- e\n");
    }

    #[test]
    fn inserted_reports_bullets_ambiguous_across_depths() {
        let text = "- x\n\t- xy\n";
        let error = inserted(text, "- e", &placed(&[], &["x"])).expect_err("ambiguous fails");
        assert_eq!(error.to_string(), "multiple bullets matching \"x\"");
    }

    #[test]
    fn inserted_scopes_bullets_to_the_section() {
        let text = "# A\n- todo\n# B\n- todo\n";
        let error = inserted(text, "- e", &placed(&[], &["todo"])).expect_err("ambiguous fails");
        assert_eq!(error.to_string(), "multiple bullets matching \"todo\"");
        let new = inserted(text, "- e", &placed(&["B"], &["todo"])).expect("insert succeeds");
        assert_eq!(new, "# A\n- todo\n# B\n- todo\n\t- e\n");
    }

    #[test]
    fn inserted_reads_every_marker() {
        let cases = [
            ("* a\n", "a", "* a\n\t- e\n"),
            ("+ a\n", "a", "+ a\n\t- e\n"),
            ("1. a\n", "a", "1. a\n\t- e\n"),
            ("1) a\n", "a", "1) a\n\t- e\n"),
            ("-   wide\n", "wide", "-   wide\n\t- e\n"),
        ];
        for (text, query, expected) in cases {
            let new = inserted(text, "- e", &placed(&[], &[query])).expect("insert succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    #[test]
    fn inserted_lands_before_a_loose_items_blank() {
        let text = "- a\n\n- b\n";
        let new = inserted(text, "- e", &placed(&[], &["a"])).expect("insert succeeds");
        assert_eq!(new, "- a\n\t- e\n\n- b\n");
    }

    #[test]
    fn inserted_extends_an_unterminated_bullet() {
        let text = "- a";
        let new = inserted(text, "- e", &placed(&[], &["a"])).expect("insert succeeds");
        assert_eq!(new, "- a\n\t- e\n");
    }

    #[test]
    fn inserted_keeps_a_crlf_note_crlf() {
        let text = "# A\r\nx\r\n# B\r\n";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\r\nx\r\n- e\r\n# B\r\n");
        let bullets = "- a\r\n- b\r\n";
        let new = inserted(bullets, "- e", &placed(&[], &["a"])).expect("insert succeeds");
        assert_eq!(new, "- a\r\n\t- e\r\n- b\r\n");
    }

    #[test]
    fn inserted_indents_every_entry_line() {
        let text = "- a\n";
        let new = inserted(text, "- e\nmore", &placed(&[], &["a"])).expect("insert succeeds");
        assert_eq!(new, "- a\n\t- e\n\tmore\n");
        let new = inserted(text, "- e\n\nmore", &placed(&[], &["a"])).expect("insert succeeds");
        assert_eq!(new, "- a\n\t- e\n\n\tmore\n");
        let new = inserted(text, "- e\r\nmore", &placed(&[], &["a"])).expect("insert succeeds");
        assert_eq!(new, "- a\n\t- e\n\tmore\n");
    }

    #[test]
    fn inserted_skips_frontmatter() {
        let text = "---\ntitle: x\n---\n# A\nbody\n";
        let new = inserted(text, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "---\ntitle: x\n---\n# A\nbody\n- e\n");
        let error = inserted(text, "- e", &placed(&["title"], &[])).expect_err("keys hidden");
        assert_eq!(error.to_string(), "no heading matching \"title\"");
        let bom = "\u{feff}# A\nx\n";
        let new = inserted(bom, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "\u{feff}# A\nx\n- e\n");
        // The block's endings carry the style when the body has none of
        // its own to show.
        let crlf = "---\r\nk: v\r\n---\r\n# A";
        let new = inserted(crlf, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "---\r\nk: v\r\n---\r\n# A\r\n- e\r\n");
    }

    /// A list entry directly above a setext heading would absorb it,
    /// and plain text directly below a list would be absorbed by it:
    /// the splice check restores the boundary with a blank line, and
    /// refuses when no blank can.
    #[test]
    fn inserted_preserves_the_structure_around_the_entry() {
        let setext = "# A\n```\nc\n```\nB\n===\ny\n";
        let new = inserted(setext, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n```\nc\n```\n- e\n\nB\n===\ny\n");
        let empty = "# A\nB\n===\n";
        let new = inserted(empty, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n- e\n\nB\n===\n");
        let list = "# A\n- item\n# B\n";
        let new = inserted(list, "plain", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n- item\n\nplain\n# B\n");
        let quoted = "- p\n  > q\n\npara\n";
        let new = inserted(quoted, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  > q\n\n\t- e\n\npara\n");
        let indented = "# A\np\n  # B\ny\n";
        let error = inserted(indented, "- e", &placed(&["A"], &[])).expect_err("no safe splice");
        assert_eq!(
            error.to_string(),
            "the entry would change the structure around it"
        );
        let inline = "# A\n*em* text\n# B\n";
        let new = inserted(inline, "- e", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n*em* text\n- e\n# B\n");
    }

    /// An entry carrying a heading of its own must not reparent the
    /// sections after it.
    #[test]
    fn inserted_guards_heading_ancestry() {
        let text = "# A\n## B\nx\n## C\ny\n";
        let error = inserted(text, "# New", &placed(&["A", "B"], &[])).expect_err("reparents C");
        assert_eq!(
            error.to_string(),
            "the entry would change the structure around it"
        );
        let deep = inserted(text, "### deep", &placed(&["A", "B"], &[])).expect("insert succeeds");
        assert_eq!(deep, "# A\n## B\nx\n### deep\n## C\ny\n");
        let tail = inserted("# A\nx\n", "## Sub", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(tail, "# A\nx\n## Sub\n");
    }

    /// Plain text under a parent with children lands as the parent's
    /// own paragraph, never as a lazy continuation of the last child.
    #[test]
    fn inserted_keeps_plain_entries_out_of_children() {
        let text = "- parent\n  - child\n";
        let new = inserted(text, "plain", &placed(&[], &["parent"])).expect("insert succeeds");
        assert_eq!(new, "- parent\n  - child\n\n  plain\n");
    }

    /// Leaf blocks the parser reports without a start tag, and entries
    /// it renders invisible, both place cleanly.
    #[test]
    fn inserted_accepts_leaf_block_entries() {
        let rule = inserted("# A\nx\n# B\n", "---", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(rule, "# A\nx\n\n---\n# B\n");
        let definition =
            inserted("# A\nx\n", "[ref]: /url", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(definition, "# A\nx\n\n[ref]: /url\n");
    }

    /// A multiline entry owns its block at its FIRST line; a later line
    /// starting one does not excuse the first being absorbed.
    #[test]
    fn inserted_requires_the_first_line_to_own_its_block() {
        let text = "# A\n- item\n# B\n";
        let new = inserted(text, "new\n- later", &placed(&["A"], &[])).expect("insert succeeds");
        assert_eq!(new, "# A\n- item\n\nnew\n- later\n# B\n");
    }

    /// An entry that would complete a half-open frontmatter fence moves
    /// the body boundary itself, and is refused rather than misjudged.
    #[test]
    fn inserted_refuses_completing_frontmatter() {
        let text = "---\n# A\nx\n";
        let error = inserted(text, "---", &placed(&["A"], &[])).expect_err("boundary moves");
        assert_eq!(
            error.to_string(),
            "the entry would change the structure around it"
        );
    }

    /// The entry lands at the end of the thread, so its sibling context
    /// is the LAST direct child, whatever the first one was indented.
    #[test]
    fn inserted_copies_the_last_direct_childs_indent() {
        let text = "- p\n    - deep\n  - shallow\n";
        let new = inserted(text, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n    - deep\n  - shallow\n  - e\n");
    }

    /// A sublist can open on its parent's marker line; the indent
    /// written beside it is column-equivalent whitespace, never a
    /// copied marker, and descent can reach it.
    #[test]
    fn inserted_handles_marker_line_sublists() {
        let outer = inserted("- - p\n", "- e", &placed(&[], &["- p"])).expect("insert succeeds");
        assert_eq!(outer, "- - p\n  - e\n");
        let inner = inserted("- - p\n", "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(inner, "- - p\n\t- e\n");
        let chain = inserted("- - beta\n", "- e", &placed(&[], &["- beta", "beta"]))
            .expect("descent succeeds");
        assert_eq!(chain, "- - beta\n\t- e\n");
    }

    /// An entry indented below an unclosed block's container ends the
    /// container — and the block with it — so the placement is safe.
    #[test]
    fn inserted_deindents_past_contained_blocks() {
        let fence = "- p\n  - c\n    ```\n    code\n";
        let new = inserted(fence, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  - c\n    ```\n    code\n  - e\n");
        let html = "- p\n  - c\n    <script>\n    raw\n";
        let new = inserted(html, "- e", &placed(&[], &["p"])).expect("insert succeeds");
        assert_eq!(new, "- p\n  - c\n    <script>\n    raw\n  - e\n");
    }

    /// Without an opening fence there is no frontmatter, and the
    /// would-be properties read as markdown: a setext heading. The
    /// contract is the frontmatter module's, applied consistently.
    #[test]
    fn inserted_reads_an_unopened_block_as_markdown() {
        let text = "title: x\n---\nbody\n";
        let new = inserted(text, "- e", &placed(&["title"], &[])).expect("insert succeeds");
        assert_eq!(new, "title: x\n---\nbody\n- e\n");
    }

    /// A placed entry must be immediately addressable through the path
    /// that placed it: the parser has to read kladde's own output as
    /// the structure it claims to have written, never as code, HTML, or
    /// a stray sibling. Asserted in both indent modes across the note
    /// shapes that make an insertion easy to get wrong.
    #[test]
    fn inserted_entries_are_addressable() {
        let cases: &[(&str, &[&str], &[&str])] = &[
            ("# A\nalpha\n# B\n", &["A"], &[]),
            ("# A\n## kladde\nx\n# B\n", &["A", "kladde"], &[]),
            ("Title\n=====\nx\n", &["Tit"], &[]),
            ("# A\n```\n# fake\n```\n# B\n", &["A"], &[]),
            ("# A\n```rust\ncode\n```", &["A"], &[]),
            ("# A\n<div>\nraw\n\n# B\n", &["A"], &[]),
            ("# A\n<script>\na\n</script>\n\n# B\n", &["A"], &[]),
            ("# A\n<!DOCTYPE html>\n# B\n", &["A"], &[]),
            ("# A\r\nx\r\n# B\r\n", &["A"], &[]),
            ("# A\n> quoted\n# B\n", &["A"], &[]),
            ("---\nt: v\n---\n# A\nx\n", &["A"], &[]),
            ("---\r\nt: v\r\n---\r\n# A", &["A"], &[]),
            ("- parent\n- other\n", &[], &["parent"]),
            ("- parent\n  - child\n", &[], &["parent"]),
            ("- a\n\t- b\n\t\t- c\n- d\n", &[], &["a", "b"]),
            ("1. p\n", &[], &["p"]),
            ("100. p\n", &[], &["p"]),
            ("-    p\n", &[], &["p"]),
            ("-     p\n", &[], &["p"]),
            ("- a\n\n- b\n", &[], &["a"]),
            ("- a", &[], &["a"]),
            ("- p\n  > q\n- q2\n", &[], &["p"]),
            ("- p\n    - deep\n  - shallow\n", &[], &["p"]),
            ("- - p\n", &[], &["p"]),
            ("- p\n  - c\n    ```\n    code\n", &[], &["p"]),
            ("- p\n  - c\n    <script>\n    raw\n", &[], &["p"]),
            ("# A\n```\nc\n```\nB\n===\ny\n", &["A"], &[]),
            ("# A\n- item\n# B\n", &["A"], &[]),
            ("- p\n  <div>\n  raw\n- q\n", &[], &["p"]),
            ("- p\n  <script>\n  a\n  </script>\n\n- q\n", &[], &["p"]),
            ("- p\n  ```\n  code\n  ```", &[], &["p"]),
            ("# A\n- todo\n# B\n- todo\n", &["B"], &["todo"]),
        ];
        for indent in [Indent::Tab, Indent::Spaces] {
            for (text, headings, bullets) in cases {
                let placement = Placement {
                    headings: headings.iter().map(|&query| query.to_owned()).collect(),
                    bullets: bullets.iter().map(|&query| query.to_owned()).collect(),
                    indent,
                };
                let placed = inserted(text, "- zz probe", &placement);
                assert!(placed.is_ok(), "for {text:?} with {indent:?}: {placed:?}");
                let new = placed.expect("asserted above");
                let mut descent = placement;
                descent.bullets.push("zz probe".to_owned());
                let again = inserted(&new, "- again", &descent);
                assert!(again.is_ok(), "for {new:?} with {indent:?}: {again:?}");
            }
        }
    }

    #[test]
    fn removed_takes_the_matched_line() {
        let text = "## A\n\n- one\n- two\n- three\n";
        let new = removed(text, "two", &placed(&["A"], &[])).expect("removal succeeds");
        assert_eq!(new, "## A\n\n- one\n- three\n");
    }

    /// Cutting the first line would promote the fence below it to byte
    /// zero, turning the remainder into a frontmatter block and moving
    /// the body boundary itself, so the removal is refused.
    #[test]
    fn removed_refuses_moving_the_body_boundary() {
        let error = removed("- drop\n---\nk: v\n---\n", "drop", &placed(&[], &[]))
            .expect_err("boundary move refuses");
        assert_eq!(
            error.to_string(),
            "the removal would change the structure around it"
        );
    }

    #[test]
    fn removed_defaults_to_the_whole_body() {
        let new = removed("- one\n- two\n", "one", &placed(&[], &[])).expect("removal succeeds");
        assert_eq!(new, "- two\n");
    }

    #[test]
    fn removed_descends_headings_and_bullets() {
        let text = "# A\n- p\n\t- keep\n\t- drop\n# B\n- p\n\t- drop\n";
        let new = removed(text, "drop", &placed(&["A"], &["p"])).expect("removal succeeds");
        assert_eq!(new, "# A\n- p\n\t- keep\n# B\n- p\n\t- drop\n");
    }

    /// A following nested sibling's span reaches back into the removed
    /// line's terminator; matching by content lead sees past that.
    #[test]
    fn removed_takes_a_leading_nested_sibling() {
        let text = "- p\n\t- drop\n\t- keep\n";
        let new = removed(text, "drop", &placed(&[], &["p"])).expect("removal succeeds");
        assert_eq!(new, "- p\n\t- keep\n");
    }

    #[test]
    fn removed_reads_every_marker() {
        let cases = [
            ("- x\n- y\n", "- x\n"),
            ("* x\n* y\n", "* x\n"),
            ("+ x\n+ y\n", "+ x\n"),
            ("1. x\n2. y\n", "1. x\n"),
            ("1) x\n2) y\n", "1) x\n"),
        ];
        for (text, expected) in cases {
            let new = removed(text, "y", &placed(&[], &[])).expect("removal succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    #[test]
    fn removed_refuses_nested_content() {
        let cases = [
            "- drop\n\t- child\n",
            "- drop\n  more text\n",
            "- drop\n\n  ```\n  code\n  ```\n",
            "- drop\n  > quoted\n",
        ];
        for text in cases {
            let error =
                removed(text, "drop", &placed(&[], &[])).expect_err("nested content refuses");
            assert_eq!(
                error.to_string(),
                "the bullet matching \"drop\" holds nested content",
                "for {text:?}"
            );
        }
    }

    /// The cut takes exactly the named line: blank lines around it,
    /// the loose list's separators, all stay.
    #[test]
    fn removed_takes_only_the_line_in_a_loose_list() {
        let text = "- a\n\n- b\n\n- c\n";
        let new = removed(text, "b", &placed(&[], &[])).expect("removal succeeds");
        assert_eq!(new, "- a\n\n\n- c\n");
    }

    #[test]
    fn removed_takes_the_only_item() {
        let text = "## A\n\n- solo\n\ntext\n";
        let new = removed(text, "solo", &placed(&["A"], &[])).expect("removal succeeds");
        assert_eq!(new, "## A\n\n\ntext\n");
    }

    /// A last item's span holds the blank after its list; the survivor
    /// absorbs that residue and the blank separator stays in the note.
    #[test]
    fn removed_takes_the_last_item_before_a_block() {
        let cases = [
            ("- a\n- zdrop\n\nparagraph\n", "- a\n\nparagraph\n"),
            (
                "## S\n\n- x\n- zdrop\n\n## Next\n",
                "## S\n\n- x\n\n## Next\n",
            ),
            ("- a\n- zdrop\n\n* other\n", "- a\n\n* other\n"),
            ("- a\n\t- zdrop\n\n> quote\n", "- a\n\n> quote\n"),
            (
                "- a\n- zdrop\n\n```\ncode\n```\n",
                "- a\n\n```\ncode\n```\n",
            ),
        ];
        for (text, expected) in cases {
            let new = removed(text, "zdrop", &placed(&[], &[])).expect("removal succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    /// The daily-note shape: retracting the last sub-bullet of a
    /// session thread keeps the blank separating session groups.
    #[test]
    fn removed_takes_the_last_nested_child() {
        let text = "## Stream\n\n- Session A\n\t- did x\n\t- did y\n\n- Session B\n\t- did z\n";
        let new =
            removed(text, "did y", &placed(&["Stream"], &["Session A"])).expect("removal succeeds");
        assert_eq!(
            new,
            "## Stream\n\n- Session A\n\t- did x\n\n- Session B\n\t- did z\n"
        );
    }

    /// A cut that would promote a nested blank line into a top-level
    /// separator reshapes untouched siblings, so it is refused.
    #[test]
    fn removed_refuses_promoting_a_nested_blank() {
        let text = "- a\n\t- b\n\n\t- b2\n\n\t- zd\n- last\n";
        let error = removed(text, "zd", &placed(&[], &[])).expect_err("loose flip refuses");
        assert_eq!(
            error.to_string(),
            "the removal would change the structure around it"
        );
    }

    /// A bullet whose own text spells a heading parses as a heading
    /// block inside the item; it vanishes with the cut like anything
    /// else on the removed line.
    #[test]
    fn removed_takes_a_bullet_spelling_a_heading() {
        let cases = [
            ("- # of retries hit 3\n- next\n", "# of retries", "- next\n"),
            ("- first\n- # ZZ\n", "# ZZ", "- first\n"),
            ("- x\n\t- ## deep\n", "## deep", "- x\n"),
        ];
        for (text, query, expected) in cases {
            let new = removed(text, query, &placed(&[], &[])).expect("removal succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    /// A container's span reaches into the next line's indentation, so
    /// blocks compare by content edges, not span edges.
    #[test]
    fn removed_reads_indent_wobble() {
        let text = "- a\n  - b\n  1. x\n";
        let new = removed(text, "b", &placed(&[], &[])).expect("removal succeeds");
        assert_eq!(new, "- a\n  1. x\n");
        let new = removed(text, "x", &placed(&[], &[])).expect("removal succeeds");
        assert_eq!(new, "- a\n  - b\n");
    }

    /// The vanished nested list's span runs into the continuation
    /// line's indent; content edges see past it.
    #[test]
    fn removed_takes_the_only_child_before_a_continuation() {
        let text = "## S\n\n- Session A\n\t- did x\n\n  wrap up line\n";
        let new = removed(text, "did x", &placed(&["S"], &[])).expect("removal succeeds");
        assert_eq!(new, "## S\n\n- Session A\n\n  wrap up line\n");
    }

    /// A surviving sibling list's span absorbs its own terminator when
    /// a differently-marked sibling vanishes; its content edge stays.
    #[test]
    fn removed_takes_a_mixed_marker_sibling() {
        let text = "- Session A\n\t1. reviewed the PR\n\t- shipped it\n";
        let new =
            removed(text, "shipped it", &placed(&[], &["Session A"])).expect("removal succeeds");
        assert_eq!(new, "- Session A\n\t1. reviewed the PR\n");
    }

    /// The blank a removed last item leaves behind would become
    /// content inside an unclosed fence or raw HTML block, so the
    /// removal is refused rather than silently editing code.
    #[test]
    fn removed_refuses_feeding_a_code_block() {
        let cases = [
            "- notes\n\t```\n\tlet a = 1;\n- scratch\n\n\n",
            "- notes\n\t<pre>x\n- scratch\n\n\n",
        ];
        for text in cases {
            let error = removed(text, "scratch", &placed(&[], &[])).expect_err("code feed refuses");
            assert_eq!(
                error.to_string(),
                "the removal would change the structure around it",
                "for {text:?}"
            );
        }
    }

    /// Cutting the first item of an ordered list promotes the next
    /// item's marker to the list's start: a sequential list renders
    /// unchanged and passes, a gapped one renumbers and is refused.
    /// Cutting a later item renumbers the rest downward, the way
    /// deleting from a numbered list does everywhere, and passes.
    #[test]
    fn removed_guards_ordered_list_starts() {
        let new = removed("1. one\n2. two\n3. three\n", "one", &placed(&[], &[]))
            .expect("sequential start succeeds");
        assert_eq!(new, "2. two\n3. three\n");
        let new = removed("1. keep\n2. drop\n3. later\n", "drop", &placed(&[], &[]))
            .expect("natural renumbering succeeds");
        assert_eq!(new, "1. keep\n3. later\n");
        let new = removed("1. drop\n1. keep\n", "drop", &placed(&[], &[]))
            .expect("an all-ones list renumbers downward");
        assert_eq!(new, "1. keep\n");
        let error = removed("1. zdrop\n5. five\n7. seven\n", "zdrop", &placed(&[], &[]))
            .expect_err("renumbering refuses");
        assert_eq!(
            error.to_string(),
            "the removal would change the structure around it"
        );
    }

    /// A bullet's own single line goes whole, whatever blocks its text
    /// parses into: a quoted child is not separately addressable, so
    /// it vanishes with its bullet the way a spelled heading does.
    #[test]
    fn removed_takes_a_bullet_quoting_content() {
        let new = removed("- > - child\n- keep\n", "> - child", &placed(&[], &[]))
            .expect("removal succeeds");
        assert_eq!(new, "- keep\n");
    }

    /// A marker-line sublist nests a child on the parent's own line:
    /// removing the parent is refused for the child it holds, and
    /// removing the child is refused for the line it shares.
    #[test]
    fn removed_refuses_marker_line_nesting() {
        let text = "- - child\n- keep\n";
        let error = removed(text, "- child", &placed(&[], &[])).expect_err("nested child refuses");
        assert_eq!(
            error.to_string(),
            "the bullet matching \"- child\" holds nested content"
        );
        let error = removed(text, "child", &placed(&[], &[])).expect_err("shared line refuses");
        assert_eq!(
            error.to_string(),
            "the bullet matching \"child\" shares its line with another bullet"
        );
    }

    /// Removing the last item of a loose list would leave a tight one,
    /// reshaping the survivor's blocks, so the removal is refused.
    #[test]
    fn removed_refuses_a_reshaping_removal() {
        let cases = ["- a\n\n- b\n", "- a\n\n- b\n\ntail\n"];
        for text in cases {
            let error = removed(text, "b", &placed(&[], &[])).expect_err("loose survivor refuses");
            assert_eq!(
                error.to_string(),
                "the removal would change the structure around it",
                "for {text:?}"
            );
        }
    }

    #[test]
    fn removed_reports_unmatched_and_ambiguous_targets() {
        let cases = [
            ("- x\n", "y", "no bullet matching \"y\""),
            ("- x\n- xy\n", "x", "multiple bullets matching \"x\""),
        ];
        for (text, query, message) in cases {
            let error = removed(text, query, &placed(&[], &[])).expect_err("bad target fails");
            assert_eq!(error.to_string(), message, "for {text:?}");
        }
        let error = removed("- x\n", "x", &placed(&["A"], &[])).expect_err("missing scope fails");
        assert_eq!(error.to_string(), "no heading matching \"A\"");
    }

    #[test]
    fn removed_validates_queries() {
        let cases: [(&[&str], &[&str], &str, &str); 3] = [
            (&["A"], &[], "", "invalid bullet \"\": it is empty"),
            (&[""], &[], "x", "invalid heading \"\": it is empty"),
            (
                &[],
                &["a\nb"],
                "x",
                "invalid bullet \"a\\nb\": it contains a line break",
            ),
        ];
        for (headings, bullets, query, message) in cases {
            let error =
                removed("- x\n", query, &placed(headings, bullets)).expect_err("invalid fails");
            assert_eq!(error.to_string(), message, "for {query:?}");
        }
    }

    #[test]
    fn removed_refuses_bare_carriage_returns() {
        let error = removed("- a\r- b\r", "b", &placed(&[], &[])).expect_err("bare CR refuses");
        assert_eq!(
            error.to_string(),
            "the note uses bare carriage-return line endings"
        );
    }

    #[test]
    fn removed_skips_frontmatter() {
        let text = "---\nk: v\n---\n- a\n- b\n";
        let new = removed(text, "b", &placed(&[], &[])).expect("removal succeeds");
        assert_eq!(new, "---\nk: v\n---\n- a\n");
    }

    #[test]
    fn removed_keeps_a_crlf_note_crlf() {
        let text = "## A\r\n\r\n- one\r\n- two\r\n";
        let new = removed(text, "two", &placed(&["A"], &[])).expect("removal succeeds");
        assert_eq!(new, "## A\r\n\r\n- one\r\n");
    }

    #[test]
    fn removed_takes_an_unterminated_last_line() {
        let new = removed("- a\n- b", "b", &placed(&[], &[])).expect("removal succeeds");
        assert_eq!(new, "- a\n");
    }

    /// A probe an insertion placed must remove back to the original
    /// note: the removal-side analogue of addressability.
    #[test]
    fn removed_round_trips_an_appended_probe() {
        let cases: [(&str, &[&str], &[&str]); 5] = [
            ("# A\nalpha\n", &["A"], &[]),
            ("- p\n\t- c\n", &[], &["p", "c"]),
            ("## A\n\n- p\nx\n", &["A"], &["p"]),
            ("#### Thoughts\n\n#### Stream\n", &["Thoughts"], &[]),
            (
                "## S\n\n- Session A\n\t- did x\n\n- Session B\n",
                &["S"],
                &["Session A"],
            ),
        ];
        for (text, headings, bullets) in cases {
            let grown =
                inserted(text, "- zq probe", &placed(headings, bullets)).expect("insert succeeds");
            let back =
                removed(&grown, "zq probe", &placed(headings, bullets)).expect("removal succeeds");
            assert_eq!(back, text, "for {text:?}");
        }
    }

    #[test]
    fn toggled_flips_the_matched_task() {
        let text = "- [ ] milk\n- [x] bob\n";
        let new = toggled(text, "milk", &placed(&[], &[]), true).expect("check succeeds");
        assert_eq!(new, "- [x] milk\n- [x] bob\n");
        let new = toggled(&new, "bob", &placed(&[], &[]), false).expect("uncheck succeeds");
        assert_eq!(new, "- [x] milk\n- [ ] bob\n");
    }

    /// The query names the task's text past the box, so the same query
    /// checks a task and unchecks it again.
    #[test]
    fn toggled_matches_past_the_box() {
        let text = "- [ ] milk\n";
        let checked = toggled(text, "milk", &placed(&[], &[]), true).expect("check succeeds");
        let back = toggled(&checked, "milk", &placed(&[], &[]), false).expect("uncheck succeeds");
        assert_eq!(back, text);
    }

    #[test]
    fn toggled_leaves_the_asked_state() {
        let cases = [
            ("- [x] t\n", true),
            ("- [X] t\n", true),
            ("- [ ] t\n", false),
        ];
        for (text, checked) in cases {
            let new = toggled(text, "t", &placed(&[], &[]), checked).expect("no-op succeeds");
            assert_eq!(new, text, "for {text:?}");
        }
    }

    #[test]
    fn toggled_reads_task_shapes() {
        let cases = [
            ("- [X] t\n", "- [ ] t\n"),
            ("* [x] t\n", "* [ ] t\n"),
            ("+ [x] t\n", "+ [ ] t\n"),
            ("1. [x] t\n", "1. [ ] t\n"),
        ];
        for (text, expected) in cases {
            let new = toggled(text, "t", &placed(&[], &[]), false).expect("uncheck succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    /// Only task bullets are candidates: a plain bullet or a bracketed
    /// line that is no box never matches, whatever its text.
    #[test]
    fn toggled_skips_non_tasks() {
        let cases = ["- t\n", "- [y] t\n", "- [ ]t\n", "- [x]t\n", "-\n"];
        for text in cases {
            let error =
                toggled(text, "t", &placed(&[], &[]), true).expect_err("non-task never matches");
            assert_eq!(error.to_string(), "no task matching \"t\"", "for {text:?}");
        }
        let new =
            toggled("- t\n- [ ] t\n", "t", &placed(&[], &[]), true).expect("the one task matches");
        assert_eq!(new, "- t\n- [x] t\n");
    }

    #[test]
    fn toggled_accepts_a_bare_box() {
        let new = toggled("- [ ]\n", "x", &placed(&[], &[]), true).expect_err("nothing matches");
        assert_eq!(new.to_string(), "no task matching \"x\"");
    }

    /// Whitespace after the box is separator, not text: a tab or extra
    /// spaces never have to be spelled in the query.
    #[test]
    fn toggled_reads_separator_whitespace() {
        let cases = [
            ("- [ ]\tship it\n", "- [x]\tship it\n"),
            ("- [ ]  ship it\n", "- [x]  ship it\n"),
            ("- [ ] \t ship it\n", "- [x] \t ship it\n"),
        ];
        for (text, expected) in cases {
            let new = toggled(text, "ship it", &placed(&[], &[]), true).expect("check succeeds");
            assert_eq!(new, expected, "for {text:?}");
        }
    }

    /// A wide gap after the marker turns the rest of the line into
    /// indented code inside the item; a box the parser reads as code
    /// is content, never a task.
    #[test]
    fn toggled_skips_a_boxed_code_sample() {
        let cases = ["-     [ ] code sample\n", "-\t\t[ ] code sample\n"];
        for text in cases {
            let error = toggled(text, "code sample", &placed(&[], &[]), true)
                .expect_err("code is never a task");
            assert_eq!(
                error.to_string(),
                "no task matching \"code sample\"",
                "for {text:?}"
            );
        }
    }

    #[test]
    fn toggled_scopes_and_descends() {
        let text = "# A\n- p\n\t- [ ] t\n# B\n- p\n\t- [ ] t\n";
        let new = toggled(text, "t", &placed(&["A"], &["p"]), true).expect("check succeeds");
        assert_eq!(new, "# A\n- p\n\t- [x] t\n# B\n- p\n\t- [ ] t\n");
    }

    #[test]
    fn toggled_reports_no_and_multiple_tasks() {
        let error = toggled("- [ ] t\n- [x] tu\n", "t", &placed(&[], &[]), true)
            .expect_err("two tasks are ambiguous");
        assert_eq!(error.to_string(), "multiple tasks matching \"t\"");
        let error =
            toggled("- [ ] t\n", "t", &placed(&["A"], &[]), true).expect_err("missing scope");
        assert_eq!(error.to_string(), "no heading matching \"A\"");
    }

    #[test]
    fn toggled_validates_queries() {
        let error = toggled("- [ ] t\n", "", &placed(&[], &[]), true).expect_err("empty query");
        assert_eq!(error.to_string(), "invalid bullet \"\": it is empty");
        let error = toggled("- [ ] t\n", "t", &placed(&["a\nb"], &["c"]), true)
            .expect_err("broken heading query");
        assert_eq!(
            error.to_string(),
            "invalid heading \"a\\nb\": it contains a line break"
        );
    }

    #[test]
    fn toggled_refuses_bare_carriage_returns() {
        let error =
            toggled("- [ ] t\r", "t", &placed(&[], &[]), true).expect_err("bare CR refuses");
        assert_eq!(
            error.to_string(),
            "the note uses bare carriage-return line endings"
        );
    }

    #[test]
    fn toggled_skips_frontmatter_and_keeps_crlf() {
        let text = "---\nk: v\n---\n- [ ] t\r\n";
        let new = toggled(text, "t", &placed(&[], &[]), true).expect("check succeeds");
        assert_eq!(new, "---\nk: v\n---\n- [x] t\r\n");
    }
}
