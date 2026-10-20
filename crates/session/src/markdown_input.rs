//! Display-only compatibility for phase labels and unparsed CJK bold delimiters.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::borrow::Cow;

pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
}

pub fn marker(text: &str) -> bool {
    matches!(text, "💭" | "✅" | "💬" | "🧑‍💻" | "❌" | "⚠️")
}

/// Put a phase label in its own parser paragraph so block syntax stays intact.
/// The output renderer joins that label to the first visible line again.
pub fn source(markdown: &str) -> Cow<'_, str> {
    let mut offsets = Vec::new();
    let mut offset = 0;
    for line in markdown.split_inclusive('\n') {
        if let Some((label, _)) = line.split_once(' ')
            && marker(label)
        {
            offsets.push((offset, offset + label.len()));
        }
        offset += line.len();
    }
    if offsets.is_empty() {
        return Cow::Borrowed(markdown);
    }
    let mut source = markdown.as_bytes().to_vec();
    for (_, separator) in &offsets {
        source[*separator] = b'\n';
    }
    let candidate = String::from_utf8(source).expect("only ASCII spaces replaced");
    let protected = Parser::new_ext(&candidate, options())
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(
                event,
                Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock) | Event::Code(_)
            )
            .then_some(range)
        })
        .collect::<Vec<_>>();
    let mut source = candidate.into_bytes();
    let mut separate = Vec::new();
    for (line, separator) in offsets {
        if protected.iter().any(|range| range.contains(&line)) {
            source[separator] = b' ';
        } else {
            separate.push(separator);
        }
    }
    let mut source = String::from_utf8(source).expect("only ASCII newlines replaced");
    for separator in separate.into_iter().rev() {
        source.insert(separator + 1, '\n');
    }
    Cow::Owned(source)
}

pub fn events(markdown: &str) -> Vec<Event<'_>> {
    bounded_events(markdown, usize::MAX).expect("unbounded parser event budget")
}

pub(super) fn bounded_events(markdown: &str, limit: usize) -> Option<Vec<Event<'_>>> {
    let mut parser = Parser::new_ext(markdown, options())
        .into_offset_iter()
        .peekable();
    let mut events = Vec::new();
    let mut code = false;
    while let Some((event, range)) = parser.next() {
        if events.len() > limit {
            return None;
        }
        match &event {
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock) => code = true,
            Event::End(TagEnd::CodeBlock | TagEnd::HtmlBlock) => code = false,
            _ => {}
        }
        if code || !matches!(event, Event::Text(_)) {
            events.push(event);
            continue;
        }
        let start = range.start;
        let mut end = range.end;
        let mut run = vec![event];
        while matches!(parser.peek(), Some((Event::Text(_), _))) {
            if events.len().saturating_add(run.len()) >= limit {
                return None;
            }
            let (event, range) = parser.next().expect("peeked text");
            end = range.end;
            run.push(event);
        }
        let literal = run
            .iter()
            .filter_map(|event| match event {
                Event::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect::<String>();
        let raw = &markdown[start..end];
        // Escapes/entities and already parsed formatting keep CommonMark semantics.
        if raw == literal && raw.contains("**") {
            bold(raw, &mut events, limit)?;
        } else {
            events.extend(run);
        }
    }
    (events.len() <= limit).then_some(events)
}

fn bold<'a>(raw: &'a str, events: &mut Vec<Event<'a>>, limit: usize) -> Option<()> {
    let markers = raw
        .match_indices("**")
        .map(|(i, _)| i)
        .filter(|i| {
            (*i == 0 || raw.as_bytes()[i - 1] != b'*') && raw.as_bytes().get(i + 2) != Some(&b'*')
        })
        .take(limit.saturating_add(1))
        .collect::<Vec<_>>();
    if markers.len() > limit {
        return None;
    }
    let mut copied = 0;
    for pair in markers.as_chunks::<2>().0 {
        let (start, end) = (pair[0], pair[1]);
        let text = &raw[start + 2..end];
        if !text
            .chars()
            .any(|c| matches!(c, '\u{2e80}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'))
            || text.trim().is_empty()
        {
            continue;
        }
        if events.len().saturating_add(6) >= limit {
            return None;
        }
        events.push(Event::Text(raw[copied..start].into()));
        let trimmed = text.trim();
        let leading = text.len() - text.trim_start().len();
        events.push(Event::Text(text[..leading].into()));
        events.push(Event::Start(Tag::Strong));
        events.push(Event::Text(trimmed.into()));
        events.push(Event::End(TagEnd::Strong));
        events.push(Event::Text(text[leading + trimmed.len()..].into()));
        copied = end + 2;
    }
    events.push(Event::Text(raw[copied..].into()));
    Some(())
}
