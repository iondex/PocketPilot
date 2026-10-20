//! Stable Twilight V2 builders carried by the existing Serenity HTTP/Gateway connection.
use crate::discord::Error;
use serde::{Deserialize, Serialize};
use serenity::http::{Http, LightMethod, Request, Route};
use twilight_model::channel::message::{
    Component, Embed, MessageFlags,
    component::{MediaGallery, MediaGalleryItem, UnfurledMediaItem},
};
use twilight_util::builder::message::{ContainerBuilder, TextDisplayBuilder};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mentions {
    pub parse: Vec<String>,
    pub users: Vec<String>,
    pub roles: Vec<String>,
    pub replied_user: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub flags: MessageFlags,
    pub components: Vec<Component>,
    pub allowed_mentions: Mentions,
}

impl Message {
    pub fn text(text: String) -> Self {
        Self::menu(text, Vec::new())
    }

    pub fn menu(text: String, controls: Vec<Component>) -> Self {
        let mut container = ContainerBuilder::new()
            .accent_color(Some(0x0058_65f2))
            .component(TextDisplayBuilder::new(text).build());
        for component in controls {
            container = container.component(component);
        }
        Self {
            flags: MessageFlags::IS_COMPONENTS_V2,
            components: vec![container.build().into()],
            allowed_mentions: Mentions {
                parse: Vec::new(),
                users: Vec::new(),
                roles: Vec::new(),
                replied_user: false,
            },
        }
    }

    pub fn with_images(text: String, images: Vec<(String, String)>) -> Self {
        let galleries = images
            .chunks(10)
            .map(|images| {
                MediaGallery {
                    id: None,
                    items: images
                        .iter()
                        .map(|(url, label)| MediaGalleryItem {
                            media: UnfurledMediaItem {
                                url: url.clone(),
                                proxy_url: None,
                                height: None,
                                width: None,
                                content_type: None,
                            },
                            description: Some(session::markdown::preview(label, 1024)),
                            spoiler: None,
                        })
                        .collect(),
                }
                .into()
            })
            .collect();
        Self::menu(text, galleries)
    }

    pub fn text_content(&self) -> String {
        fn text(components: &[Component]) -> Vec<String> {
            components
                .iter()
                .flat_map(|component| match component {
                    Component::Container(container) => text(&container.components),
                    Component::TextDisplay(display) => vec![display.content.clone()],
                    _ => Vec::new(),
                })
                .collect()
        }
        text(&self.components).join("\n\n")
    }
}

/// Old frozen strings keep their original classic mode across an upgrade.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Part {
    Classic(String),
    V2(Message),
}

#[derive(Serialize)]
struct Edit<'a> {
    #[serde(flatten)]
    message: &'a Message,
    content: Option<String>,
    embeds: Vec<Embed>,
}

pub async fn edit(http: &Http, route: Route<'_>, message: &Message) -> Result<(), Error> {
    // Clearing both traditional fields is required when upgrading an existing message to V2.
    let payload = Edit {
        message,
        content: None,
        embeds: Vec::new(),
    };
    request(http, route, LightMethod::Patch, &payload).await?;
    Ok(())
}

#[derive(Deserialize)]
struct Receipt {
    id: serenity::all::MessageId,
}

pub async fn post(http: &Http, route: Route<'_>, message: &Message) -> Result<u64, Error> {
    let response = request(http, route, LightMethod::Post, message).await?;
    let receipt: Receipt = response.json().await.map_err(|_| Error::Request)?;
    Ok(receipt.id.get())
}

async fn request(
    http: &Http,
    route: Route<'_>,
    method: LightMethod,
    payload: &impl Serialize,
) -> Result<reqwest::Response, Error> {
    let body = serde_json::to_vec(payload).map_err(|_| Error::Backend)?;
    // The existing rate limiter, explicit proxy and token routing remain authoritative.
    http.request(Request::new(route, method).body(Some(body)))
        .await
        .map_err(|_| Error::Request)
}
