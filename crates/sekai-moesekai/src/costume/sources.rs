//! What the `costume-role` and `costume` assets are built from: the game data tables, the assets
//! of the version, and the motions of each role.

use std::collections::BTreeMap;

use sekai_convert::ConvertError;
use sekai_convert::tables::TableContext;
use sekai_format::costume::Colors;

use super::tables::{self, Figure, Tables};

const MODEL: &str = "costume-model";
const TEXTURE: &str = "texture";
const MOTION: &str = "costume-motion";

/// Where the model bundles are: the newer directory first, then the older one.
const MODEL_ROOTS: &[&str] = &["live_pv/model/characterv2/", "live_pv/model/character/"];

/// The suffixes of the colour, shadow and value texture of a colour variation.
const COLOR_SUFFIX: &str = "_C";
const SHADOW_SUFFIX: &str = "_S";
const VALUE_SUFFIX: &str = "_H";

/// The clips of the costume screen, by the name of their member.
const CLIPS: &[&str] = &["motion", "face", "motion_loop", "face_loop"];
/// The motion bundles of the costume screen.
const MOTION_ROOT: &str = "character/motion/costume_setting/";
/// The figure whose motions are the ladies' motions; any other figure takes the men's.
const LADIES: &str = "ladies";
const MEN: &str = "mens";

/// The tables and the assets of the version.
pub(super) struct Sources {
    pub(super) tables: Tables,
    pub(super) version: Version,
}

impl Sources {
    /// Reads the tables and lists the bundles of the version.
    pub(super) fn read(cx: &dyn TableContext) -> Result<Self, ConvertError> {
        Ok(Self {
            tables: tables::load(cx)?,
            version: Version::read(cx),
        })
    }

    /// Clip name to the name of the `costume-motion` asset that holds it, for the first motion
    /// bundle of the version; empty when no bundle is in the version.
    pub(super) fn motions(
        &self,
        cx: &dyn TableContext,
        unit_ids: &[i64],
        figure: &Figure,
    ) -> Result<BTreeMap<String, String>, ConvertError> {
        let of_rows = unit_ids.iter().map(|id| format!("{MOTION_ROOT}{id:02}_00"));
        let of_figure = format!("{MOTION_ROOT}{}", figure_bundle(figure));
        let bundle = match of_rows
            .chain([of_figure])
            .find(|bundle| !self.version.assets(bundle, MOTION).is_empty())
        {
            Some(bundle) => bundle,
            None => return Ok(BTreeMap::new()),
        };
        let mut motions = BTreeMap::new();
        for asset in self.version.assets(&bundle, MOTION) {
            for member in cx.document(asset)?.members {
                let clip = member.role.to_lowercase();
                if CLIPS.contains(&clip.as_str()) {
                    motions.entry(clip).or_insert_with(|| asset.clone());
                }
            }
        }
        Ok(motions)
    }
}

/// The motion bundle of a figure.
fn figure_bundle(figure: &Figure) -> &'static str {
    if figure.figure == LADIES { LADIES } else { MEN }
}

/// The assets of the version, by bundle and form.
pub(super) struct Version {
    bundles: BTreeMap<String, BTreeMap<&'static str, Vec<String>>>,
}

impl Version {
    pub(super) fn read(cx: &dyn TableContext) -> Self {
        let bundles = cx.bundles().into_iter().map(|bundle| {
            let mut forms: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
            for entry in cx.assets(bundle) {
                forms
                    .entry(entry.form)
                    .or_default()
                    .push(entry.name.clone());
            }
            (bundle.to_owned(), forms)
        });
        Self {
            bundles: bundles.collect(),
        }
    }

    /// The names of the assets of `form` that `bundle` holds, in name order.
    pub(super) fn assets(&self, bundle: &str, form: &str) -> &[String] {
        self.bundles
            .get(bundle)
            .and_then(|forms| forms.get(form))
            .map_or(&[], Vec::as_slice)
    }

    /// The only `costume-model` asset of a bundle below a model directory, the newer directory
    /// first.
    pub(super) fn model(&self, bundle: &str) -> Option<String> {
        self.under_roots(bundle, |bundle| match self.assets(bundle, MODEL) {
            [one] => Some(one.clone()),
            _ => None,
        })
    }

    /// The colour textures of a bundle below a model directory, the newer directory first.
    pub(super) fn colors(&self, bundle: &str) -> Option<Colors> {
        self.under_roots(bundle, |bundle| self.colors_of(bundle))
    }

    /// The colour textures of `bundle`, when it holds one texture for each suffix.
    fn colors_of(&self, bundle: &str) -> Option<Colors> {
        let texture = |suffix: &str| {
            self.assets(bundle, TEXTURE)
                .iter()
                .find(|name| name.ends_with(suffix))
                .cloned()
        };
        Some(Colors {
            color: Some(texture(COLOR_SUFFIX)?),
            shadow: Some(texture(SHADOW_SUFFIX)?),
            value: Some(texture(VALUE_SUFFIX)?),
        })
    }

    /// What `find` gives for the first of the model directories that holds `bundle`.
    fn under_roots<T>(&self, bundle: &str, find: impl Fn(&str) -> Option<T>) -> Option<T> {
        MODEL_ROOTS
            .iter()
            .map(|root| format!("{root}{bundle}"))
            .find_map(|bundle| find(&bundle))
    }
}
