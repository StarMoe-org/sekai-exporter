//! A client package as the source of `update --client`, named by the version of its application
//! and its content.

use sekai_format::identity::{AssetIdentity, ClientIdentity, VersionIdentity};
use sekai_game::cdn::{BundleEntry, Source, SourceError};
use sekai_game::sink::Sink;
use sekai_unity::index::{FileLayouts, OpenFile};
use sekai_unity::{Bundle, ClassLayouts, NoBlockCache, SerializedFile};
use sha2::{Digest, Sha256};

use super::{Bundles, Held, Package};

/// The class ID of `PlayerSettings`.
const PLAYER_SETTINGS: i32 = 129;
/// The file of the player data that holds the player settings.
const GLOBAL_GAME_MANAGERS: &str = "globalgamemanagers";

/// The client package `package` of a region as the supply of an update.
pub(crate) fn update_source(
    package: &sekai_game::cdn::Package,
) -> Result<crate::update::Supply, String> {
    let scripts = package.script_table.as_deref();
    let Package { bundles, layouts } = Package::open(&package.path, &package.class_table, scripts)?;
    let player = bundles.player();
    let bundle = Bundle::open(player.bytes.clone(), player.sha256)
        .map_err(|e| format!("the player data: {e}"))?;
    let application_version = application_version(&bundle, &layouts)?;
    let source = ClientSource {
        bundles,
        application_version,
    };
    Ok(crate::update::Supply {
        source: Box::new(source),
        classes: Box::new(layouts),
    })
}

/// A client package as the source of an update: its bundles, as a version named by the version
/// of the application.
struct ClientSource {
    bundles: Bundles,
    application_version: String,
}

impl ClientSource {
    /// The content of the package: the SHA-256 of a line `<name> <SHA-256>` for each of its
    /// bundles, as lowercase hexadecimal.
    fn content(&self) -> String {
        let line = |bundle: &Held| format!("{} {}\n", bundle.name, bundle.content());
        let lines: String = self.bundles.0.iter().map(line).collect();
        hex(&Sha256::digest(lines.as_bytes()))
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
        let entry = |bundle: &Held| BundleEntry {
            name: bundle.name.to_owned(),
            declared: bundle.content(),
            download_path: None,
            size: bundle.bytes.len() as u64,
            dependencies: Vec::new(),
        };
        Ok(self.bundles.0.iter().map(entry).collect())
    }

    fn fetch(
        &self,
        _identity: &VersionIdentity,
        entry: &BundleEntry,
        sink: &mut dyn Sink,
    ) -> Result<u64, SourceError> {
        let bundle = self.bundles.get(&entry.name).ok_or_else(|| {
            let message = format!("the package has no bundle {}", entry.name);
            SourceError::new("missing_bundle", message)
        })?;
        let bytes = bundle.bytes.as_slice();
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

impl Held {
    /// The SHA-256 of the bundle, as lowercase hexadecimal.
    fn content(&self) -> String {
        hex(&self.sha256)
    }
}

impl Bundles {
    /// The bundle of the player data.
    fn player(&self) -> &Held {
        &self.0[0]
    }
}

/// `bytes` as lowercase hexadecimal.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
