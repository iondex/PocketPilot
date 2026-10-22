//! Keep public Markdown links checked by Cargo, including images and heading anchors.
use percent_encoding::percent_decode_str;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

fn markdown_files(directory: &Path, recursive: bool, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if recursive && path.is_dir() {
            markdown_files(&path, true, files);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }
}

fn anchors(text: &str) -> HashSet<String> {
    let mut result = HashSet::new();
    let mut counts = HashMap::<String, usize>::new();
    let mut heading = None::<String>;
    for event in Parser::new_ext(text, Options::all()) {
        match event {
            Event::Start(Tag::Heading { .. }) => heading = Some(String::new()),
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = &mut heading {
                    heading.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                let base: String = heading
                    .take()
                    .unwrap()
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))
                    .map(|c| if c == ' ' { '-' } else { c })
                    .collect();
                let count = counts.entry(base.clone()).or_default();
                result.insert(if *count == 0 {
                    base
                } else {
                    format!("{base}-{count}")
                });
                *count += 1;
            }
            _ => {}
        }
    }
    result
}

fn local_target(root: &Path, source: &Path, target: &str) -> Result<(), String> {
    // A URI scheme or network-path reference is outside this offline check.
    if target.starts_with("//")
        || target.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        })
    {
        return Ok(());
    }
    let (path, fragment) = target.split_once('#').unwrap_or((target, ""));
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    let path = percent_decode_str(path)
        .decode_utf8()
        .map_err(|e| e.to_string())?;
    let destination = if path.is_empty() {
        source.to_path_buf()
    } else {
        source.parent().unwrap().join(path.as_ref())
    };
    let destination = destination
        .canonicalize()
        .map_err(|_| "target does not exist")?;
    if !destination.starts_with(root) {
        return Err("link escapes the repository".into());
    }
    if !fragment.is_empty()
        && destination
            .extension()
            .is_some_and(|extension| extension == "md")
    {
        let fragment = percent_decode_str(fragment)
            .decode_utf8()
            .map_err(|e| e.to_string())?;
        let text = fs::read_to_string(destination).map_err(|e| e.to_string())?;
        if !anchors(&text).contains(fragment.as_ref()) {
            return Err("heading does not exist".into());
        }
    }
    Ok(())
}

#[test]
fn public_markdown_links_resolve_within_the_repository() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut sources = Vec::new();
    markdown_files(&root, false, &mut sources);
    for directory in ["docs", "assets", ".github"] {
        markdown_files(&root.join(directory), true, &mut sources);
    }
    sources.sort();
    let mut errors = Vec::new();
    for source in sources {
        let text = fs::read_to_string(&source).unwrap();
        // Parsing Markdown excludes fenced examples and supports reference-style links.
        for (event, range) in Parser::new_ext(&text, Options::all()).into_offset_iter() {
            if let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = event
                && let Err(reason) = local_target(&root, &source, &dest_url)
            {
                let line = text[..range.start].bytes().filter(|b| *b == b'\n').count() + 1;
                errors.push(format!(
                    "{}:{line}: {reason}: {dest_url}",
                    source.strip_prefix(&root).unwrap().display()
                ));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn markdown_anchor_rules_and_local_path_boundaries_are_checked() {
    assert_eq!(
        anchors("# 中文 `Code`\n\n```md\n# Ignored\n```\n\n# 中文 Code\n"),
        HashSet::from(["中文-code".into(), "中文-code-1".into()])
    );
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let source = root.join("source.md");
    fs::write(&source, "# Guide\n").unwrap();
    fs::write(root.join("with space.md"), "# Target\n").unwrap();
    assert!(local_target(&root, &source, "with%20space.md#target").is_ok());
    assert!(local_target(&root, &source, "#missing").is_err());
    assert!(local_target(&root, &source, "missing.md").is_err());
    assert!(local_target(&root, &source, "../").is_err());
    assert!(local_target(&root, &source, "https://example.com/guide").is_ok());
}
