//! The assets that a run builds from game data tables.

use std::collections::BTreeMap;
use std::io;

use sekai_catalog::Key;
use sekai_convert::tables::{AssetEntry, TableConverter};
use sekai_pipeline::claims::Claims;
use sekai_pipeline::state::TableOutput;
use sekai_pipeline::tables::{Built, TableStage, VersionAssets};
use sekai_publish::Binding;

/// The assets built from tables, or kept from the previous run, with the produced assets that
/// `claims` own as the assets of the version, whose forms are among `forms` and whose documents
/// `document` finds.
pub(super) fn build_tables(
    stage: &TableStage<'_>,
    claims: &[Claims],
    forms: &[&'static str],
    document: &(dyn Fn(&str) -> io::Result<Option<Key>> + Sync),
) -> Result<Vec<Built>, String> {
    let mut by_bundle: BTreeMap<String, Vec<AssetEntry>> = BTreeMap::new();
    for claims in claims {
        let Some(owner) = claims.owner().filter(|owner| !owner.failed) else {
            continue;
        };
        let form = forms
            .iter()
            .copied()
            .find(|&form| form == owner.form)
            .ok_or_else(|| format!("{} has the unknown form {}", claims.name, owner.form))?;
        by_bundle
            .entry(owner.unit.clone())
            .or_default()
            .push(AssetEntry {
                name: claims.name.clone(),
                form,
            });
    }
    let version = VersionAssets {
        by_bundle,
        document,
    };
    let built = sekai_pipeline::tables::build(stage, &version).map_err(|e| e.to_string())?;
    let kept = built.iter().filter(|built| built.kept).count();
    println!("table assets {} kept {kept}", built.len());
    let owned = |name: &str| {
        let found = claims.binary_search_by(|claims| claims.name.as_str().cmp(name));
        found.is_ok_and(|index| claims[index].owner().is_some())
    };
    match built.iter().find(|built| owned(&built.output.name)) {
        Some(built) => Err(format!("two assets are named {}", built.output.name)),
        None => Ok(built),
    }
}

/// The bindings of the assets built from tables that the state lists.
pub(super) fn table_bindings(tables: &[TableOutput]) -> Result<Vec<Binding>, String> {
    tables
        .iter()
        .map(|table| {
            let document = table.document.parse().map_err(|_| {
                format!(
                    "the table asset {} names the document {}",
                    table.name, table.document
                )
            })?;
            Ok(Binding {
                name: table.name.clone(),
                document,
            })
        })
        .collect()
}

/// The converters that build assets from game data tables.
pub(super) fn table_converters() -> Vec<Box<dyn TableConverter>> {
    let mut converters = episode_converters();
    converters.extend(costume_converters());
    converters.extend(mysekai_converters());
    converters
}

#[cfg(feature = "costume")]
fn costume_converters() -> Vec<Box<dyn TableConverter>> {
    vec![
        Box::new(sekai_moesekai::costume::CostumeRoles),
        Box::new(sekai_moesekai::costume::Costumes),
    ]
}

#[cfg(not(feature = "costume"))]
fn costume_converters() -> Vec<Box<dyn TableConverter>> {
    Vec::new()
}

#[cfg(feature = "episode")]
fn episode_converters() -> Vec<Box<dyn TableConverter>> {
    vec![Box::new(sekai_moesekai::episode::Episode)]
}

#[cfg(not(feature = "episode"))]
fn episode_converters() -> Vec<Box<dyn TableConverter>> {
    Vec::new()
}

#[cfg(feature = "mysekai")]
fn mysekai_converters() -> Vec<Box<dyn TableConverter>> {
    vec![
        Box::new(sekai_mysekai::alone::AloneActions),
        Box::new(sekai_mysekai::characters::Characters),
        Box::new(sekai_mysekai::client_config::ClientConfig),
        Box::new(sekai_mysekai::communication::CommunicationActions),
        Box::new(sekai_mysekai::emoticon::Emoticons),
        Box::new(sekai_mysekai::facial::FacialTables),
        Box::new(sekai_mysekai::fixture_areas::FixtureAreas),
        Box::new(sekai_mysekai::fixture_attach_points::FixtureAttachPoints),
        Box::new(sekai_mysekai::fixtures::Fixtures),
        Box::new(sekai_mysekai::talks::FixtureTalks),
        Box::new(sekai_mysekai::talks::Talks),
    ]
}

#[cfg(not(feature = "mysekai"))]
fn mysekai_converters() -> Vec<Box<dyn TableConverter>> {
    Vec::new()
}
