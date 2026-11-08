//! CRI `@UTF` tables, the structure of ACB files.

use crate::DecodeError;

const HEADER_SIZE: usize = 0x20;
const FLAG_NAME: u8 = 0x10;
const FLAG_DEFAULT: u8 = 0x20;
const FLAG_ROW: u8 = 0x40;

fn malformed(message: impl Into<String>) -> DecodeError {
    DecodeError::Malformed {
        what: "@UTF table",
        message: message.into(),
    }
}

/// Slices `data`, reporting a range past the end as malformed.
fn range(data: &[u8], offset: usize, length: usize) -> Result<&[u8], DecodeError> {
    offset
        .checked_add(length)
        .and_then(|end| data.get(offset..end))
        .ok_or_else(|| malformed("a value lies past the end of its section"))
}

fn be_uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |value, &b| value << 8 | u64::from(b))
}

fn read_string(strings: &[u8], offset: usize) -> Result<&str, DecodeError> {
    let tail = strings
        .get(offset..)
        .ok_or_else(|| malformed("a string lies past the end of its section"))?;
    let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    core::str::from_utf8(&tail[..end]).map_err(|_| malformed("a string is not UTF-8"))
}

fn value_size(kind: u8) -> Result<usize, DecodeError> {
    match kind {
        0 | 1 => Ok(1),
        2 | 3 => Ok(2),
        4 | 5 | 8 | 10 => Ok(4),
        6 | 7 | 11 => Ok(8),
        other => Err(DecodeError::Unsupported {
            what: "@UTF column type",
            value: other.into(),
        }),
    }
}

/// A value of a table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Value<'a> {
    Int(i64),
    Float(f32),
    Text(&'a str),
    Data(&'a [u8]),
}

struct Column<'a> {
    name: &'a str,
    kind: u8,
    source: Source,
}

#[derive(Clone, Copy)]
enum Source {
    Absent,
    Default(usize),
    Row(usize),
}

/// One table, reading its values from the bytes it was opened on.
pub(super) struct Table<'a> {
    schema: &'a [u8],
    rows_section: &'a [u8],
    strings: &'a [u8],
    data: &'a [u8],
    columns: Vec<Column<'a>>,
    row_width: usize,
    rows: usize,
}

impl<'a> Table<'a> {
    pub(super) fn open(table: &'a [u8]) -> Result<Self, DecodeError> {
        if table.len() < HEADER_SIZE {
            return Err(DecodeError::Truncated {
                what: "@UTF table",
                needed: HEADER_SIZE,
                available: table.len(),
            });
        }
        if &table[..4] != b"@UTF" {
            return Err(malformed("no @UTF signature"));
        }
        let size = be_uint(&table[4..8]) as usize + 8;
        if table.len() < size {
            return Err(DecodeError::Truncated {
                what: "@UTF table",
                needed: size,
                available: table.len(),
            });
        }
        let rows_offset = be_uint(&table[0x0A..0x0C]) as usize + 8;
        let strings_offset = be_uint(&table[0x0C..0x10]) as usize + 8;
        let data_offset = be_uint(&table[0x10..0x14]) as usize + 8;
        let column_count = be_uint(&table[0x18..0x1A]) as usize;
        let row_width = be_uint(&table[0x1A..0x1C]) as usize;
        let rows = be_uint(&table[0x1C..0x20]) as usize;
        if !(HEADER_SIZE <= rows_offset
            && rows_offset <= strings_offset
            && strings_offset <= data_offset
            && data_offset <= size)
        {
            return Err(malformed("sections are out of order"));
        }
        let rows_section = &table[rows_offset..strings_offset];
        if rows
            .checked_mul(row_width)
            .is_none_or(|total| total > rows_section.len())
        {
            return Err(malformed("rows exceed their section"));
        }
        let schema = &table[HEADER_SIZE..rows_offset];
        let strings = &table[strings_offset..data_offset];

        let mut columns = Vec::with_capacity(column_count);
        let (mut position, mut row_position) = (0, 0);
        for _ in 0..column_count {
            let info = range(schema, position, 5)?;
            let flags = info[0] & 0xF0;
            let kind = info[0] & 0x0F;
            if flags & FLAG_NAME == 0 {
                return Err(malformed("a column has no name"));
            }
            let name = read_string(strings, be_uint(&info[1..5]) as usize)?;
            position += 5;
            let size = value_size(kind)?;
            let source = if flags & FLAG_DEFAULT != 0 {
                let source = Source::Default(position);
                position += size;
                source
            } else if flags & FLAG_ROW != 0 {
                let source = Source::Row(row_position);
                row_position += size;
                source
            } else {
                Source::Absent
            };
            columns.push(Column { name, kind, source });
        }
        Ok(Self {
            schema,
            rows_section,
            strings,
            data: &table[data_offset..size],
            columns,
            row_width,
            rows,
        })
    }

    pub(super) fn rows(&self) -> usize {
        self.rows
    }

    /// The value of a column in a row, or `None` if the table has no such column or stores
    /// nothing for it.
    pub(super) fn value(&self, row: usize, name: &str) -> Result<Option<Value<'a>>, DecodeError> {
        if row >= self.rows {
            return Err(malformed("row index is past the last row"));
        }
        let Some(column) = self.columns.iter().find(|c| c.name == name) else {
            return Ok(None);
        };
        let size = value_size(column.kind)?;
        let bytes = match column.source {
            Source::Absent => return Ok(None),
            Source::Default(offset) => range(self.schema, offset, size)?,
            // A row can reach past its width into the rows that follow; tables written by
            // version 3 tools do this for trailing columns.
            Source::Row(offset) => range(self.rows_section, row * self.row_width + offset, size)?,
        };
        let raw = be_uint(bytes);
        Ok(Some(match column.kind {
            0 | 2 | 4 | 6 => Value::Int(raw as i64),
            1 => Value::Int(i64::from(raw as u8 as i8)),
            3 => Value::Int(i64::from(raw as u16 as i16)),
            5 => Value::Int(i64::from(raw as u32 as i32)),
            7 => Value::Int(raw as i64),
            8 => Value::Float(f32::from_bits(raw as u32)),
            10 => Value::Text(read_string(self.strings, raw as usize)?),
            _ => Value::Data(range(
                self.data,
                be_uint(&bytes[..4]) as usize,
                be_uint(&bytes[4..]) as usize,
            )?),
        }))
    }

    pub(super) fn int(&self, row: usize, name: &str) -> Result<Option<i64>, DecodeError> {
        match self.value(row, name)? {
            Some(Value::Int(value)) => Ok(Some(value)),
            Some(_) => Err(malformed(format!("column {name} is not an integer"))),
            None => Ok(None),
        }
    }

    pub(super) fn text(&self, row: usize, name: &str) -> Result<Option<&'a str>, DecodeError> {
        match self.value(row, name)? {
            Some(Value::Text(value)) => Ok(Some(value)),
            Some(_) => Err(malformed(format!("column {name} is not a string"))),
            None => Ok(None),
        }
    }

    pub(super) fn data(&self, row: usize, name: &str) -> Result<Option<&'a [u8]>, DecodeError> {
        match self.value(row, name)? {
            Some(Value::Data(value)) => Ok(Some(value)),
            Some(_) => Err(malformed(format!("column {name} is not binary data"))),
            None => Ok(None),
        }
    }

    /// Opens the table stored in a binary column, if the column holds any bytes.
    pub(super) fn table(&self, row: usize, name: &str) -> Result<Option<Table<'a>>, DecodeError> {
        match self.data(row, name)? {
            Some(bytes) if !bytes.is_empty() => Table::open(bytes).map(Some),
            _ => Ok(None),
        }
    }
}
