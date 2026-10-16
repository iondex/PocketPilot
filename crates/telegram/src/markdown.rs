//! CommonMark/GFM to Telegram's supported HTML, with escaped raw HTML and URLs.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

#[derive(Default)]
pub struct Rendered {
    pub html: String,
    pub plain: String,
}

impl Rendered {
    fn text(&mut self, value: &str) {
        self.html.push_str(&teloxide::utils::html::escape(value));
        self.plain.push_str(value);
    }

    fn mark(&mut self, value: &str) {
        self.html.push_str(value);
    }

    fn newline(&mut self) {
        if !self.plain.is_empty() && !self.plain.ends_with('\n') {
            self.text("\n");
        }
    }

    fn append(&mut self, other: &Self) {
        self.html.push_str(&other.html);
        self.plain.push_str(&other.plain);
    }
}

pub fn render(markdown: &str) -> Rendered {
    let parser = Parser::new_ext(
        markdown,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    );
    let mut out = Rendered::default();
    let mut links = Vec::new();
    let mut list = Vec::<Option<u64>>::new();
    let mut table: Option<Table> = None;
    for event in parser {
        if matches!(event, Event::Start(Tag::Table(_))) {
            out.newline();
            table = Some(Table::default());
            continue;
        }
        if matches!(event, Event::End(TagEnd::Table)) {
            if let Some(table) = table.take() {
                table.write(&mut out);
            }
            continue;
        }
        if let Some(t) = table.as_mut() {
            match event {
                Event::Start(Tag::TableHead) => t.head = true,
                Event::End(TagEnd::TableHead) => t.head = false,
                Event::Start(Tag::TableCell) => t.cell = Rendered::default(),
                Event::End(TagEnd::TableCell) => {
                    if t.head {
                        t.headers.push(std::mem::take(&mut t.cell));
                    } else {
                        t.row.push(std::mem::take(&mut t.cell));
                    }
                }
                Event::End(TagEnd::TableRow) => {
                    t.rows.push(std::mem::take(&mut t.row));
                }
                other => inline(other, &mut t.cell, &mut links),
            }
            continue;
        }
        match event {
            Event::Start(Tag::Paragraph) => out.newline(),
            Event::End(TagEnd::Paragraph) => {
                out.newline();
                out.text("\n");
            }
            Event::Start(Tag::Heading { .. }) => {
                out.newline();
                out.mark("<b>");
            }
            Event::End(TagEnd::Heading(_)) => {
                out.mark("</b>");
                out.newline();
            }
            Event::Start(Tag::CodeBlock(_)) => {
                out.newline();
                out.mark("<pre>");
            }
            Event::End(TagEnd::CodeBlock) => {
                out.mark("</pre>");
                out.newline();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                out.newline();
                out.mark("<blockquote>");
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                out.mark("</blockquote>");
                out.newline();
            }
            Event::Start(Tag::List(start)) => {
                out.newline();
                list.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                list.pop();
                out.newline();
            }
            Event::Start(Tag::Item) => {
                out.newline();
                if let Some(Some(n)) = list.last_mut() {
                    let label = format!("{n}. ");
                    *n += 1;
                    out.text(&label);
                } else {
                    out.text("• ");
                }
            }
            Event::End(TagEnd::Item) => out.newline(),
            Event::Rule => {
                out.newline();
                out.text("────────\n");
            }
            other => inline(other, &mut out, &mut links),
        }
    }
    while out.plain.ends_with('\n') {
        out.plain.pop();
        if out.html.ends_with('\n') {
            out.html.pop();
        } else {
            break;
        }
    }
    out
}

fn inline(event: Event<'_>, out: &mut Rendered, links: &mut Vec<bool>) {
    match event {
        Event::Start(Tag::Emphasis) => out.mark("<i>"),
        Event::End(TagEnd::Emphasis) => out.mark("</i>"),
        Event::Start(Tag::Strong) => out.mark("<b>"),
        Event::End(TagEnd::Strong) => out.mark("</b>"),
        Event::Start(Tag::Strikethrough) => out.mark("<s>"),
        Event::End(TagEnd::Strikethrough) => out.mark("</s>"),
        Event::Start(Tag::Link { dest_url, .. }) => {
            let allowed = url::Url::parse(&dest_url)
                .is_ok_and(|u| matches!(u.scheme(), "https" | "http" | "tg" | "mailto"));
            links.push(allowed);
            if allowed {
                out.mark("<a href=\"");
                out.mark(&teloxide::utils::html::escape(&dest_url).replace('"', "&quot;"));
                out.mark("\">");
            }
        }
        Event::End(TagEnd::Link) => {
            if links.pop() == Some(true) {
                out.mark("</a>");
            }
        }
        Event::Start(Tag::Image { dest_url, .. }) => {
            out.text("🖼 ");
            inline(
                Event::Start(Tag::Link {
                    link_type: pulldown_cmark::LinkType::Inline,
                    dest_url,
                    title: "".into(),
                    id: "".into(),
                }),
                out,
                links,
            );
        }
        Event::End(TagEnd::Image) => inline(Event::End(TagEnd::Link), out, links),
        Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => out.text(&text),
        Event::Code(text) => {
            out.mark("<code>");
            out.text(&text);
            out.mark("</code>");
        }
        Event::SoftBreak | Event::HardBreak => out.text("\n"),
        Event::TaskListMarker(done) => out.text(if done { "☑ " } else { "☐ " }),
        Event::FootnoteReference(text) => {
            out.text("[");
            out.text(&text);
            out.text("]");
        }
        _ => {}
    }
}

#[derive(Default)]
struct Table {
    head: bool,
    headers: Vec<Rendered>,
    rows: Vec<Vec<Rendered>>,
    row: Vec<Rendered>,
    cell: Rendered,
}

impl Table {
    fn write(self, out: &mut Rendered) {
        if let Some(grid) = self.compact_grid() {
            out.mark("<pre>");
            out.text(&grid);
            out.mark("</pre>");
            out.text("\n\n");
            return;
        }
        for (row_index, row) in self.rows.iter().enumerate() {
            for (index, cell) in row.iter().enumerate() {
                if index == 0 {
                    out.text(&format!("{}. ", row_index + 1));
                } else {
                    out.text("  ");
                }
                if let Some(header) = self
                    .headers
                    .get(index)
                    .filter(|h| !h.plain.trim().is_empty())
                {
                    out.mark("<b>");
                    out.text(header.plain.trim());
                    out.mark("</b>");
                    out.text(": ");
                }
                if cell.plain.trim().is_empty() {
                    out.text("—");
                } else {
                    out.append(cell);
                }
                out.newline();
            }
            out.text("\n");
        }
    }

    fn compact_grid(&self) -> Option<String> {
        let columns = self.headers.len();
        if !(1..=3).contains(&columns) {
            return None;
        }
        let mut widths = vec![0; columns];
        for row in std::iter::once(&self.headers).chain(self.rows.iter()) {
            if row.len() != columns {
                return None;
            }
            for (index, cell) in row.iter().enumerate() {
                // Preserve clickable links and inline emphasis using row cards.
                if cell.plain.contains('\n')
                    || cell.html != teloxide::utils::html::escape(&cell.plain)
                {
                    return None;
                }
                widths[index] = widths[index].max(cell.plain.trim().width());
            }
        }
        if widths.iter().sum::<usize>() + 3 * (columns - 1) > 38 {
            return None;
        }
        let line = |cells: &[Rendered]| {
            cells
                .iter()
                .zip(&widths)
                .map(|(cell, width)| {
                    let text = cell.plain.trim();
                    format!("{text}{}", " ".repeat(width - text.width()))
                })
                .collect::<Vec<_>>()
                .join(" │ ")
        };
        let mut grid = line(&self.headers);
        grid.push('\n');
        grid.push_str(
            &widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┼─"),
        );
        for row in &self.rows {
            grid.push('\n');
            grid.push_str(&line(row));
        }
        Some(grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_tables_align_cjk_and_numbers_within_a_phone_width() {
        let r = render("| 名称 | 数量 |\n|---|---:|\n| 中文 | 12 |\n| Rust | 3 |");
        assert!(r.html.starts_with("<pre>"));
        let lines = r.plain.trim_matches('\n').lines().collect::<Vec<_>>();
        let widths = lines.iter().map(|line| line.width()).collect::<Vec<_>>();
        assert!(
            widths
                .iter()
                .all(|width| *width == widths[0] && *width <= 38)
        );
        assert!(r.plain.contains("中文") && r.plain.contains("Rust"));
    }

    #[test]
    fn wide_or_rich_tables_become_numbered_cards_with_first_column_labels_and_links() {
        let r = render(
            "| 名称 | 说明 | 下载 |\n|---|---|---|\n| App | 这是一段较长的中文说明以验证不会产生横向滚动或截断 | [打开](https://example.com) |",
        );
        assert!(!r.html.contains("<pre>"));
        assert!(r.html.contains("1. <b>名称</b>: App"));
        assert!(r.html.contains("<b>说明</b>"));
        assert!(r.html.contains("<a href=\"https://example.com\">打开</a>"));
        assert!(!r.plain.contains('|'));
    }

    #[test]
    fn renders_agent_markdown_and_tables_without_unsafe_html_or_broken_utf16() {
        let r = render(
            "**推荐** [网站](https://example.com?a=1&b=2)\n\n| 客户端 | 平台 |\n|---|---|\n| **JellyCine** | 安卓 |\n\n<script>x</script>\n\n```rs\nlet x = a < b;\n```",
        );
        assert!(r.html.contains("<b>推荐</b>"));
        assert!(r.html.contains("<b>JellyCine</b>"));
        assert!(r.html.contains("<b>平台</b>: 安卓"));
        assert!(r.html.contains("&amp;b=2"));
        assert!(!r.html.contains("<table>"));
        assert!(!r.html.contains("<script>"));
        assert!(r.html.contains("&lt;script&gt;"));
        assert!(r.html.contains("<pre>let x = a &lt; b;"));
        assert!(!render("[link](javascript:alert(1))").html.contains("href="));
    }
}
