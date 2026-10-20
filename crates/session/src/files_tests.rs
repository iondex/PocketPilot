use super::*;

#[test]
fn uploaded_paths_are_context_while_chat_history_keeps_only_names() {
    assert_eq!(name("../../\n../../notes.txt"), "notes.txt");
    assert_eq!(name("../.."), "file");
    assert_eq!(name("🌈.png"), "file.png");
    let long = name(&format!("{}.png", "图".repeat(100)));
    assert!(long.len() <= 200 && long.ends_with(".png"));
    let files = [Uploaded {
        name: "image.png".into(),
        path: "/private/uploads/image.png".into(),
        image: true,
    }];
    let raw = envelope("Describe the image", &files);
    let content = crate::content::user_content(&raw, &[]);
    assert_eq!(content.text, "Describe the image");
    assert_eq!(content.attachments[0].name, "image.png");
    assert!(!content.text.contains("/private"));
    let item = crate::TextItem {
        turn_id: "turn".into(),
        item_id: "file".into(),
        role: crate::Role::User,
        phase: None,
        text: raw,
        complete: true,
        client_id: None,
        attachments: vec![],
    };
    let shown = crate::content::display(&item);
    assert!(shown.contains("image.png"));
    assert!(!shown.contains("/private"));
    assert!(image_mime("image/png"));
    assert!(!image_mime("image/svg+xml"));
}
