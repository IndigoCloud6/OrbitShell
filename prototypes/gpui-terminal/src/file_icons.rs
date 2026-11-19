use std::{collections::HashMap, sync::LazyLock};

use gpui::{ImageSource, IntoElement, Resource, Styled, img, px};
use rust_embed::RustEmbed;
use serde::Deserialize;

const PREFIX: &str = "material/";

#[derive(RustEmbed)]
#[folder = "assets/material/"]
#[prefix = "material/"]
#[include = "*.svg"]
pub struct MaterialAssets;

type Associations = HashMap<String, String>;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rules {
    file_names: Associations,
    file_extensions: Associations,
    folder_names: Associations,
}

#[derive(Default, Deserialize)]
struct Manifest {
    icons: Associations,
    #[serde(flatten)]
    base: Rules,
    light: Rules,
}

impl Manifest {
    fn lookup(&self, field: fn(&Rules) -> &Associations, key: &str) -> Option<&str> {
        // The file panel uses a light surface even when an individual terminal is dark.
        field(&self.light)
            .get(key)
            .or_else(|| field(&self.base).get(key))
            .map(String::as_str)
    }

    fn icon_id(&self, name: &str, directory: bool) -> &str {
        if directory {
            return self
                .lookup(|rules| &rules.folder_names, name)
                .unwrap_or("folder");
        }
        self.lookup(|rules| &rules.file_names, name)
            .or_else(|| name.starts_with(".env.").then_some("settings"))
            .or_else(|| {
                // Compare each suffix from longest to shortest.
                name.match_indices('.').find_map(|(dot, _)| {
                    self.lookup(|rules| &rules.file_extensions, &name[dot + 1..])
                })
            })
            .unwrap_or("file")
    }
}

static MANIFEST: LazyLock<Manifest> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/material/associations.json")).unwrap_or_default()
});

pub fn file_icon(name: &str, directory: bool) -> impl IntoElement {
    let id = MANIFEST.icon_id(&name.to_lowercase(), directory);
    let fallback = if directory {
        "material/folder.svg"
    } else {
        "material/file.svg"
    };
    let path = MANIFEST
        .icons
        .get(id)
        .map(|filename| format!("{PREFIX}{filename}"))
        .filter(|path| MaterialAssets::get(path).is_some())
        .unwrap_or_else(|| fallback.into());

    img(ImageSource::Resource(Resource::Embedded(path.into())))
        .size(px(17.))
        .flex_none()
}
