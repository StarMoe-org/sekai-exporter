//! What the forms built from scripts share: finding the scripts of a package, reading the random
//! draw of their loops and writing a runtime document.

use std::fmt::Display;

use sekai_convert::tables::{TableAsset, TableContext, TablePlan};
use sekai_convert::{Asset, Content, ConvertError, Member};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::constants::{Constants, binding};
use crate::lua::{self, BinaryOp, Block, Expr, Number, Stat};

const TEXT: &str = "text";
const TEXT_MEMBER: &str = "file";
const DEFINES: &str = "defines";
const LIBRARY: &str = "lib";

/// The role of the one member of a runtime document.
pub(crate) const DOCUMENT: &str = "document";

const RANDOM_VARIABLE: &str = "rand";

/// The range of the draw that opens a loop.
pub(crate) const RANDOM_RANGE: RandomRange = RandomRange { min: 0, max: 99 };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct RandomRange {
    pub(crate) min: i64,
    pub(crate) max: i64,
}

pub(crate) fn malformed(e: impl Display) -> ConvertError {
    ConvertError::new("malformed", e.to_string())
}

/// The rows of the game data table `name`.
pub(crate) fn rows<T: DeserializeOwned>(
    cx: &dyn TableContext,
    name: &str,
) -> Result<Vec<T>, ConvertError> {
    serde_json::from_slice(&cx.table(name)?).map_err(|e| malformed(format!("{name}: {e}")))
}

/// Whether `expr` is the plain call `name(...)`.
pub(crate) fn is_call(expr: &Expr, name: &str) -> bool {
    matches!(expr, Expr::Call(call) if call.name() == Some(name))
}

/// The bound of a condition `rand < bound`.
pub(crate) fn random_below(condition: &Expr) -> Option<i64> {
    let Expr::Binary(BinaryOp::Less, left, right) = condition else {
        return None;
    };
    match (left.as_ref(), right.as_ref()) {
        (Expr::Name(name), Expr::Number(Number::Int(bound))) if name == RANDOM_VARIABLE => {
            Some(*bound)
        }
        _ => None,
    }
}

/// The range of the statement `rand = math.random(min, max)`, local or not.
pub(crate) fn random_draw(statement: &Stat) -> Option<RandomRange> {
    let (target, Expr::Call(call)) = binding(statement)? else {
        return None;
    };
    let Expr::Field(module, function) = &call.callee else {
        return None;
    };
    let random = target == RANDOM_VARIABLE
        && **module == Expr::Name("math".to_owned())
        && function == "random";
    match call.arguments.as_slice() {
        [
            Expr::Number(Number::Int(min)),
            Expr::Number(Number::Int(max)),
        ] if random => Some(RandomRange {
            min: *min,
            max: *max,
        }),
        _ => None,
    }
}

/// The scripts of one package.
#[derive(Clone, Copy)]
pub(crate) struct Scripts<'a> {
    cx: &'a dyn TableContext,
    bundle: &'a str,
}

impl<'a> Scripts<'a> {
    /// The scripts of `bundle`, if the version holds any.
    pub(crate) fn open(cx: &'a dyn TableContext, bundle: &'a str) -> Option<Self> {
        cx.assets(bundle)
            .iter()
            .any(|entry| entry.form == TEXT)
            .then_some(Self { cx, bundle })
    }

    fn asset(&self, name: &str) -> String {
        format!("{}/{name}.lua", self.bundle)
    }

    /// Whether the package holds the script `name`, given without its extension.
    pub(crate) fn holds(&self, name: &str) -> bool {
        let asset = self.asset(name);
        self.cx
            .assets(self.bundle)
            .binary_search_by(|entry| entry.name.cmp(&asset))
            .is_ok()
    }

    /// The syntax tree of the script `name`, given without its extension.
    pub(crate) fn parse(&self, name: &str) -> Result<Block, ConvertError> {
        let asset = self.asset(name);
        let bytes = self.cx.member(&asset, TEXT_MEMBER)?;
        let source = String::from_utf8(bytes).map_err(|e| malformed(format!("{asset}: {e}")))?;
        lua::parse(&source).map_err(|e| malformed(format!("{asset}: {e}")))
    }

    /// The constants of the package.
    pub(crate) fn defines(&self) -> Result<Constants, ConvertError> {
        Ok(Constants::read(&self.parse(DEFINES)?))
    }

    /// The constants of the library of the package.
    pub(crate) fn library(&self) -> Result<Constants, ConvertError> {
        Ok(Constants::read(&self.parse(LIBRARY)?))
    }
}

/// A plan of one asset, or of none when the version holds no sources for it.
pub(crate) struct OneAsset<F> {
    names: Vec<String>,
    build: F,
}

impl<F> OneAsset<F>
where
    F: Fn() -> Result<Asset, ConvertError> + Sync,
{
    pub(crate) fn new(name: &str, build: F) -> Self {
        Self {
            names: vec![name.to_owned()],
            build,
        }
    }
}

impl<F> TablePlan for OneAsset<F>
where
    F: Fn() -> Result<Asset, ConvertError> + Sync,
{
    fn names(&self) -> &[String] {
        &self.names
    }

    fn convert(&self, _index: usize) -> Result<TableAsset, ConvertError> {
        Ok(TableAsset {
            asset: (self.build)()?,
            references: Vec::new(),
        })
    }
}

/// A plan with no assets.
pub(crate) struct NoAssets;

impl TablePlan for NoAssets {
    fn names(&self) -> &[String] {
        &[]
    }

    fn convert(&self, index: usize) -> Result<TableAsset, ConvertError> {
        Err(ConvertError::new(
            "missing_asset",
            format!("no asset {index}"),
        ))
    }
}

/// An asset of one JSON document.
pub(crate) fn document(value: &impl Serialize) -> Result<Asset, ConvertError> {
    Ok(Asset {
        members: vec![Member {
            role: DOCUMENT.to_owned(),
            content: Content::File {
                extension: "json".to_owned(),
                bytes: serde_json::to_vec(value).map_err(malformed)?,
            },
        }],
    })
}
