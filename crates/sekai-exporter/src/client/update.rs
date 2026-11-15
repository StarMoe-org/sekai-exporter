//! A client package as the source of `update --client`, named by the version of its application
//! and its content.

use sekai_format::identity::{AssetIdentity, ClientIdentity, VersionIdentity};
use sekai_game::cdn::{BundleEntry, Source, SourceError};
use sekai_game::client::PLAYER_BUNDLE;
use sekai_game::sink::Sink;
use sekai_unity::index::{FileLayouts, OpenFile};
use sekai_unity::{Bundle, ClassLayouts, NoBlockCache, SerializedFile};

use super::{Package, Player};

/// The class ID of `PlayerSettings`.
const PLAYER_SETTINGS: i32 = 129;
/// The file of the player data that holds the player settings.
const GLOBAL_GAME_MANAGERS: &str = "globalgamemanagers";

/// The client package `package` of a region as the supply of an update.
pub(crate) fn update_source(
    package: &sekai_game::cdn::Package,
) -> Result<crate::update::Supply, String> {
    let scripts = package.script_table.as_deref();
    let Package { player, layouts } = Package::open(&package.path, &package.class_table, scripts)?;
    let bundle = Bundle::open(player.bytes.clone(), player.sha256)
        .map_err(|e| format!("the player data: {e}"))?;
    let application_version = application_version(&bundle, &layouts)?;
    let source = ClientSource {
        player,
        application_version,
    };
    Ok(crate::update::Supply {
        source: Box::new(source),
        classes: Box::new(layouts),
    })
}

/// A client package as the source of an update: its player data, as one bundle of a version named
/// by the version of the application.
struct ClientSource {
    player: Player,
    application_version: String,
}

impl ClientSource {
    /// The SHA-256 of the player data, as lowercase hexadecimal.
    fn content(&self) -> String {
        self.player
            .sha256
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

impl Source for ClientSource {
    fn identity(
        &self,
        _previous: Option<&VersionIdentity>,
    ) -> Result<VersionIdentity, SourceError> {
        Ok(VersionIdentity {
            asset: AssetIdentity::Client {
                application_version: self.application_version.clone(),
                value: self.content(),
            },
            client: ClientIdentity {
                application_version: self.application_version.clone(),
                application_hash: None,
                multiplayer_version: None,
            },
            tables: None,
        })
    }

    fn manifest(&self, _identity: &VersionIdentity) -> Result<Vec<BundleEntry>, SourceError> {
        Ok(vec![BundleEntry {
            name: PLAYER_BUNDLE.to_owned(),
            declared: self.content(),
            download_path: None,
            size: self.player.bytes.len() as u64,
            dependencies: Vec::new(),
        }])
    }

    fn fetch(
        &self,
        _identity: &VersionIdentity,
        _entry: &BundleEntry,
        sink: &mut dyn Sink,
    ) -> Result<u64, SourceError> {
        let bytes = self.player.bytes.as_slice();
        sink.write_all(bytes)
            .map_err(|e| SourceError::new("sink", e))?;
        Ok(bytes.len() as u64)
    }
}

/// The version of the application whose player data `bundle` holds: the `bundleVersion` of its
/// player settings, read with the layouts of `classes`.
fn application_version(bundle: &Bundle, classes: &dyn ClassLayouts) -> Result<String, String> {
    let failed = |e: &dyn std::fmt::Display| format!("the player settings: {e}");
    let index = bundle
        .entries()
        .iter()
        .position(|entry| entry.path == GLOBAL_GAME_MANAGERS)
        .ok_or_else(|| failed(&format!("the player data has no {GLOBAL_GAME_MANAGERS}")))?;
    let entry = bundle.entry(index, &NoBlockCache);
    let file = SerializedFile::parse(&entry).map_err(|e| failed(&e))?;
    let settings = file
        .objects
        .iter()
        .position(|object| object.class_id == PLAYER_SETTINGS)
        .ok_or_else(|| failed(&format!("{GLOBAL_GAME_MANAGERS} holds none")))?;
    let layouts = FileLayouts::with_classes(&file, classes);
    let open = OpenFile {
        path: GLOBAL_GAME_MANAGERS,
        file: &file,
        source: &entry,
        layouts: &layouts,
    };
    let value = open.value(settings).map_err(|e| failed(&e))?;
    let version = value.get("bundleVersion").and_then(|v| v.as_str());
    version
        .map(str::to_owned)
        .ok_or_else(|| failed(&"bundleVersion is missing or is not a string"))
}
