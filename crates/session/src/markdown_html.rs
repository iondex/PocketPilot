//! Recognize only small, balanced formatting tags in parsed HTML nodes, never executable HTML.
use super::{Kind, Node};

pub(super) fn normalize(nodes: Vec<Node>) -> Vec<Node> {
    let nodes = nodes
        .into_iter()
        .map(|node| match node {
            Node::Group(kind, children) if !matches!(kind, Kind::CodeBlock(_)) => {
                Node::Group(kind, normalize(children))
            }
            other => other,
        })
        .collect();
    details(inline(nodes))
}

fn tag(raw: &str) -> Option<(&'static str, Kind, bool)> {
    let raw = raw.trim();
    for (name, kind) in [
        ("b", Kind::Strong),
        ("strong", Kind::Strong),
        ("i", Kind::Emphasis),
        ("em", Kind::Emphasis),
        ("s", Kind::Strike),
        ("del", Kind::Strike),
        ("u", Kind::Underline),
        ("sub", Kind::Subscript),
        ("sup", Kind::Superscript),
        ("mark", Kind::Marked),
        ("tg-spoiler", Kind::Spoiler),
    ] {
        if raw == format!("<{name}>") {
            return Some((name, kind, false));
        }
        if raw == format!("</{name}>") {
            return Some((name, kind, true));
        }
    }
    None
}

fn inline(nodes: Vec<Node>) -> Vec<Node> {
    let mut result = Vec::new();
    let mut stack: Vec<(&str, Kind, Node, Vec<Node>)> = Vec::new();
    for node in nodes {
        if let Node::RawHtml(raw) = &node
            && let Some((name, kind, closing)) = tag(raw)
        {
            if !closing && stack.len() < 32 {
                stack.push((name, kind, node, Vec::new()));
                continue;
            }
            if closing && stack.last().is_some_and(|(opened, ..)| *opened == name) {
                let (_, kind, _, children) = stack.pop().expect("matched open tag");
                let group = Node::Group(kind, children);
                if let Some((_, _, _, children)) = stack.last_mut() {
                    children.push(group);
                } else {
                    result.push(group);
                }
                continue;
            }
        }
        if let Some((_, _, _, children)) = stack.last_mut() {
            children.push(node);
        } else {
            result.push(node);
        }
    }
    // Unclosed markup remains visible. It must not consume the rest of a message.
    while let Some((_, _, opening, children)) = stack.pop() {
        let nodes = std::iter::once(opening).chain(children);
        if let Some((_, _, _, parent)) = stack.last_mut() {
            parent.extend(nodes);
        } else {
            result.extend(nodes);
        }
    }
    result
}

fn raw(node: &Node) -> Option<String> {
    match node {
        Node::RawHtml(text) => Some(text.clone()),
        Node::Group(Kind::Paragraph, children)
            if children
                .iter()
                .all(|child| matches!(child, Node::RawHtml(_) | Node::Break)) =>
        {
            Some(
                children
                    .iter()
                    .map(|child| match child {
                        Node::RawHtml(text) => text.as_str(),
                        _ => "\n",
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

struct Disclosure {
    original: Vec<Node>,
    summary: String,
    open: bool,
    children: Vec<Node>,
}

fn summary(raw: &str) -> Option<&str> {
    raw.trim()
        .strip_prefix("<summary>")?
        .strip_suffix("</summary>")
        .map(str::trim)
}

fn details(nodes: Vec<Node>) -> Vec<Node> {
    let mut result = Vec::new();
    let mut stack: Vec<Disclosure> = Vec::new();
    for node in nodes {
        let markup = raw(&node);
        if let Some(markup) = markup.as_deref() {
            let markup = markup.trim();
            let opening = markup
                .strip_prefix("<details>")
                .map(|rest| (false, rest))
                .or_else(|| {
                    markup
                        .strip_prefix("<details open>")
                        .map(|rest| (true, rest))
                });
            if let Some((open, rest)) = opening
                && (rest.trim().is_empty() || summary(rest).is_some())
                && stack.len() < 16
            {
                stack.push(Disclosure {
                    original: vec![node],
                    summary: summary(rest).unwrap_or("…").into(),
                    open,
                    children: Vec::new(),
                });
                continue;
            }
            if markup == "</details>" && !stack.is_empty() {
                let disclosure = stack.pop().expect("matched details");
                let group = Node::Group(
                    Kind::Details {
                        summary: disclosure.summary,
                        open: disclosure.open,
                    },
                    disclosure.children,
                );
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(group);
                } else {
                    result.push(group);
                }
                continue;
            }
            if let Some(title) = summary(markup)
                && let Some(parent) = stack.last_mut().filter(|parent| parent.children.is_empty())
            {
                parent.summary = title.into();
                parent.original.push(node);
                continue;
            }
        }
        if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
        } else {
            result.push(node);
        }
    }
    while let Some(disclosure) = stack.pop() {
        let nodes = disclosure.original.into_iter().chain(disclosure.children);
        if let Some(parent) = stack.last_mut() {
            parent.children.extend(nodes);
        } else {
            result.extend(nodes);
        }
    }
    result
}
