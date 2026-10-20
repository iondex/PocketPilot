//! Split semantic blocks within message budgets without losing text or inline structure.
use super::{Document, Kind, Node, fits, plain, size};

pub(super) fn documents(nodes: &[Node], units: usize, budget: usize) -> Vec<Document> {
    let mut output = Vec::new();
    let mut current = Vec::new();
    for node in nodes {
        for fragment in fragments(node, units, budget) {
            current.push(fragment);
            if current.len() > 1 && (!fits(&plain(&current), units) || size(&current) > budget) {
                let last = current.pop().expect("just appended");
                output.push(Document {
                    nodes: std::mem::take(&mut current),
                });
                current.push(last);
            }
        }
    }
    if !current.is_empty() {
        output.push(Document { nodes: current });
    }
    output
}

fn fragments(node: &Node, units: usize, budget: usize) -> Vec<Node> {
    if fits(&plain(std::slice::from_ref(node)), units) && size(std::slice::from_ref(node)) <= budget
    {
        return vec![node.clone()];
    }
    match node {
        Node::Group(Kind::Table(alignments), children) => {
            let header = children
                .iter()
                .find(|n| matches!(n, Node::Group(Kind::TableHead, _)))
                .cloned();
            let mut rows = header.iter().cloned().collect::<Vec<_>>();
            let mut output = Vec::new();
            for row in children
                .iter()
                .filter(|n| !matches!(n, Node::Group(Kind::TableHead, _)))
            {
                rows.push(row.clone());
                let candidate = Node::Group(Kind::Table(alignments.clone()), rows.clone());
                if fits(&plain(std::slice::from_ref(&candidate)), units)
                    && size(std::slice::from_ref(&candidate)) <= budget
                {
                    continue;
                }
                let last = rows.pop().expect("just appended row");
                if rows.len() > usize::from(header.is_some()) {
                    output.push(Node::Group(
                        Kind::Table(alignments.clone()),
                        std::mem::take(&mut rows),
                    ));
                }
                rows = header.iter().cloned().collect();
                rows.push(last);
                let single = Node::Group(Kind::Table(alignments.clone()), rows.clone());
                if !fits(&plain(std::slice::from_ref(&single)), units)
                    || size(std::slice::from_ref(&single)) > budget
                {
                    for paragraph in super::table_rows(&rows) {
                        output.extend(fragments(&paragraph, units, budget));
                    }
                    rows = header.iter().cloned().collect();
                }
            }
            if rows.len() > usize::from(header.is_some()) {
                output.push(Node::Group(Kind::Table(alignments.clone()), rows));
            }
            if output.is_empty() {
                for paragraph in super::table_rows(children) {
                    output.extend(fragments(&paragraph, units, budget));
                }
            }
            output
        }
        Node::Text(text) => split_text(text, units)
            .into_iter()
            .map(Node::Text)
            .collect(),
        Node::Code(text) => split_text(text, units)
            .into_iter()
            .map(Node::Code)
            .collect(),
        Node::Group(Kind::List(start), items) if budget > 3 => items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                let start = start.map(|n| n.saturating_add(index as u64));
                fragments(item, units.saturating_sub(16), budget - 1)
                    .into_iter()
                    .map(move |item| Node::Group(Kind::List(start), vec![item]))
            })
            .collect(),
        Node::Group(kind, children)
            if budget > 3
                && units > 64
                && !matches!(kind, Kind::Link(Some(url)) | Kind::Image(Some(url)) if !fits(url, units / 2)) =>
        {
            documents(children, units - 16, budget - 1)
                .into_iter()
                .map(|doc| Node::Group(kind.clone(), doc.nodes))
                .collect()
        }
        _ => split_text(&plain(std::slice::from_ref(node)), units)
            .into_iter()
            .map(Node::Text)
            .collect(),
    }
}

/// Prefer grapheme boundaries. A single oversized grapheme must still fit the API.
pub fn split_text(text: &str, limit: usize) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let limit = limit.max(2);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let count = grapheme.encode_utf16().count();
        if used + count > limit && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            used = 0;
        }
        if count > limit {
            for c in grapheme.chars() {
                if used + c.len_utf16() > limit && !current.is_empty() {
                    chunks.push(std::mem::take(&mut current));
                    used = 0;
                }
                current.push(c);
                used += c.len_utf16();
            }
        } else {
            current.push_str(grapheme);
            used += count;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}
