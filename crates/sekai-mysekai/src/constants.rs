//! The constants that the Lua scripts of a package define and name.

use std::collections::HashMap;
use std::fmt;

use serde::Serialize;

use crate::lua::{Expr, Number, Stat, TableItem};

/// A value a script writes or names.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum Literal {
    Nil,
    Bool(bool),
    Number(Number),
    Text(String),
}

impl Literal {
    /// Whether Lua takes the value as true.
    pub(crate) fn is_truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Bool(false))
    }
}

/// The value of an argument and the constant that named it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Argument {
    pub(crate) value: Literal,
    pub(crate) alias: Option<String>,
}

/// An argument that has no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConstantError {
    /// A key that the table it names does not define.
    UndefinedKey(String),
    /// An expression that is neither a literal nor a name.
    NotAValue,
}

impl fmt::Display for ConstantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndefinedKey(token) => write!(f, "{token} is not defined"),
            Self::NotAValue => f.write_str("an argument is not a value"),
        }
    }
}

/// The value of a literal expression.
fn literal(expr: &Expr) -> Option<Literal> {
    match expr {
        Expr::Nil => Some(Literal::Nil),
        Expr::Bool(value) => Some(Literal::Bool(*value)),
        Expr::Number(value) => Some(Literal::Number(*value)),
        Expr::Str(value) => Some(Literal::Text(value.clone())),
        _ => None,
    }
}

/// The name and value that a statement binds, if it binds one name to one value.
pub(crate) fn binding(statement: &Stat) -> Option<(&str, &Expr)> {
    match statement {
        Stat::Local(names, values) => match (names.as_slice(), values.as_slice()) {
            ([name], [value]) => Some((name, value)),
            _ => None,
        },
        Stat::Assign(targets, values) => match (targets.as_slice(), values.as_slice()) {
            ([Expr::Name(name)], [value]) => Some((name, value)),
            _ => None,
        },
        _ => None,
    }
}

/// The entries of a table whose items are all names bound to literals.
fn constant_table(items: &[TableItem]) -> Option<HashMap<String, Literal>> {
    items
        .iter()
        .map(|item| match item {
            TableItem::Named(key, value) => Some((key.clone(), literal(value)?)),
            _ => None,
        })
        .collect()
}

/// The tables and scalars that the top level of a script defines.
#[derive(Debug, Clone, Default)]
pub(crate) struct Constants {
    tables: HashMap<String, HashMap<String, Literal>>,
    scalars: HashMap<String, Literal>,
}

impl Constants {
    /// Reads the constants of `script`, in order: a binding sees the ones before it. Other
    /// statements define no constant and are passed over.
    pub(crate) fn read(script: &[Stat]) -> Self {
        let mut constants = Self::default();
        for (name, value) in script.iter().filter_map(binding) {
            if let Expr::Table(items) = value {
                if let Some(table) = constant_table(items) {
                    constants.tables.insert(name.to_owned(), table);
                }
            } else if let Some(scalar) = constants.constant(value) {
                constants.scalars.insert(name.to_owned(), scalar);
            }
        }
        constants
    }

    /// The value of the scalar `name`.
    pub(crate) fn scalar(&self, name: &str) -> Option<&Literal> {
        self.scalars.get(name)
    }

    /// The value of a literal or of a constant.
    fn constant(&self, expr: &Expr) -> Option<Literal> {
        let argument = self.resolve(expr).ok()?;
        (literal(expr).is_some() || argument.alias.is_some()).then_some(argument.value)
    }

    /// The value of the expression `table.key`.
    fn member(&self, table: &Expr, key: &str) -> Result<Argument, ConstantError> {
        let Expr::Name(table) = table else {
            return Err(ConstantError::NotAValue);
        };
        let token = format!("{table}.{key}");
        let Some(entries) = self.tables.get(table) else {
            return Ok(Argument {
                value: Literal::Text(token),
                alias: None,
            });
        };
        match entries.get(key) {
            Some(value) => Ok(Argument {
                value: value.clone(),
                alias: Some(token),
            }),
            None => Err(ConstantError::UndefinedKey(token)),
        }
    }

    /// The value of an argument. A name that no constant defines stands for itself.
    pub(crate) fn resolve(&self, expr: &Expr) -> Result<Argument, ConstantError> {
        if let Some(value) = literal(expr) {
            return Ok(Argument { value, alias: None });
        }
        match expr {
            Expr::Name(name) => Ok(match self.scalars.get(name) {
                Some(value) => Argument {
                    value: value.clone(),
                    alias: Some(name.clone()),
                },
                None => Argument {
                    value: Literal::Text(name.clone()),
                    alias: None,
                },
            }),
            Expr::Field(table, key) => self.member(table, key),
            _ => Err(ConstantError::NotAValue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::parse;

    fn constants(source: &str) -> Constants {
        Constants::read(&parse(source).unwrap())
    }

    fn argument(constants: &Constants, source: &str) -> Result<Argument, ConstantError> {
        let block = parse(&format!("x = {source}")).unwrap();
        let [Stat::Assign(_, values)] = block.as_slice() else {
            panic!("{block:?}");
        };
        constants.resolve(&values[0])
    }

    const DEFINES: &str = "blend = 0.5\nMotionType = { _s = 0, _l = 1 }\nL = MotionType._l\n\
                           local local_value = 2\nfunction f() end\nmixed = { 1, a = 2 }\n";

    #[test]
    fn a_member_of_a_table_takes_its_value_and_alias() {
        let constants = constants(DEFINES);
        assert_eq!(
            argument(&constants, "MotionType._s").unwrap(),
            Argument {
                value: Literal::Number(Number::Int(0)),
                alias: Some("MotionType._s".to_owned()),
            }
        );
    }

    #[test]
    fn a_scalar_may_name_a_member_of_an_earlier_table() {
        let constants = constants(DEFINES);
        assert_eq!(
            constants.scalar("L"),
            Some(&Literal::Number(Number::Int(1)))
        );
        assert_eq!(
            constants.scalar("local_value"),
            Some(&Literal::Number(Number::Int(2)))
        );
        assert_eq!(
            argument(&constants, "L").unwrap().alias.as_deref(),
            Some("L")
        );
    }

    #[test]
    fn a_key_the_table_does_not_define_is_an_error() {
        assert_eq!(
            argument(&constants(DEFINES), "MotionType._x"),
            Err(ConstantError::UndefinedKey("MotionType._x".to_owned()))
        );
    }

    #[test]
    fn a_name_no_constant_defines_stands_for_itself() {
        let constants = constants(DEFINES);
        for token in ["unknown", "Other.key"] {
            assert_eq!(
                argument(&constants, token).unwrap(),
                Argument {
                    value: Literal::Text(token.to_owned()),
                    alias: None,
                }
            );
        }
    }

    #[test]
    fn a_table_with_other_items_is_not_a_constant_table() {
        assert_eq!(
            argument(&constants(DEFINES), "mixed.a").unwrap().alias,
            None
        );
    }

    #[test]
    fn an_expression_is_not_a_value() {
        assert_eq!(
            argument(&constants(DEFINES), "1 + 2"),
            Err(ConstantError::NotAValue)
        );
    }
}
