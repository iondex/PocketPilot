//! CommonMark/GFM to Telegram's supported HTML, with escaped raw HTML and URLs.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

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
        if !self.plain.ends_with('\n') {
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
        for row in self.rows {
            for (index, cell) in row.iter().enumerate() {
                if index > 0
                    && let Some(header) = self.headers.get(index)
                {
                    out.mark("<b>");
                    out.text(header.plain.trim());
                    out.mark("</b>");
                    out.text(": ");
                }
                out.append(cell);
                out.newline();
            }
            out.text("\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
