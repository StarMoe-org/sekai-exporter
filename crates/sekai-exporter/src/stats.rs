//! `sekai-exporter stats`: where the work of a run went, from the cost table of its catalog.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use sekai_catalog::LocalStore;
use sekai_catalog::db::{Catalog, CostRow};

const MIB: f64 = (1u64 << 20) as f64;
const DEFAULT_TOP: usize = 10;

#[derive(Default)]
struct Kind {
    count: u64,
    wall_us: u64,
    cpu_us: u64,
    read: u64,
    written: u64,
}

/// How far an estimate is from the decoded size: at least 1, and 1 when they agree.
fn deviation(row: &CostRow) -> Option<f64> {
    let (estimated, decoded) = (row.estimated_memory, row.decoded_memory);
    (estimated > 0 && decoded > 0)
        .then(|| estimated.max(decoded) as f64 / estimated.min(decoded) as f64)
}

/// The report of `rows`, which are in key order.
fn report(rows: &[CostRow], top: usize) -> String {
    let mut kinds: BTreeMap<&str, Kind> = BTreeMap::new();
    let mut deviations: BTreeMap<&str, Vec<(f64, &str)>> = BTreeMap::new();
    for row in rows {
        let kind = kinds.entry(&row.kind).or_default();
        kind.count += 1;
        kind.wall_us += row.wall_us;
        kind.cpu_us += row.cpu_us;
        kind.read += row.read;
        kind.written += row.written;
        if let Some(value) = deviation(row) {
            deviations
                .entry(&row.kind)
                .or_default()
                .push((value, &row.subject));
        }
    }
    let mut out = String::from("kind\tcount\twall_s\tcpu_s\tread_mib\twritten_mib\n");
    let mut by_cost: Vec<_> = kinds.iter().collect();
    by_cost.sort_by_key(|(name, kind)| (std::cmp::Reverse(kind.cpu_us), **name));
    for (name, kind) in by_cost.iter().take(top) {
        out.push_str(&format!(
            "{name}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\n",
            kind.count,
            kind.wall_us as f64 / 1e6,
            kind.cpu_us as f64 / 1e6,
            kind.read as f64 / MIB,
            kind.written as f64 / MIB,
        ));
    }
    out.push_str("\nkind\tmeasured\tmean_deviation\tworst_deviation\tworst_subject\n");
    let mut summaries: Vec<_> = deviations
        .iter()
        .map(|(name, values)| {
            let mean = values.iter().map(|(v, _)| v).sum::<f64>() / values.len() as f64;
            let (worst, subject) =
                values.iter().fold(
                    (0.0, ""),
                    |best, (v, s)| if *v > best.0 { (*v, *s) } else { best },
                );
            (*name, values.len(), mean, worst, subject)
        })
        .collect();
    summaries.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(b.0)));
    for (name, measured, mean, worst, subject) in summaries.iter().take(top) {
        out.push_str(&format!(
            "{name}\t{measured}\t{mean:.2}\t{worst:.2}\t{subject}\n"
        ));
    }
    out
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut output = None;
    let mut top = DEFAULT_TOP;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("top") => top = parser.value()?.parse()?,
            Value(v) if output.is_none() => output = Some(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    let output = output.ok_or("expected <output>")?;
    let store = LocalStore::open(output.join("store")).map_err(|e| e.to_string())?;
    let catalog = Catalog::open_or_rebuild(&output.join("catalog.redb"), &store)
        .map_err(|e| e.to_string())?;
    let rows = catalog.costs().map_err(|e| e.to_string())?;
    print!("{}", report(&rows, top));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, subject: &str, cpu_us: u64, estimated: u64, decoded: u64) -> CostRow {
        CostRow {
            kind: kind.to_owned(),
            subject: subject.to_owned(),
            wall_us: cpu_us,
            cpu_us,
            read: 1 << 20,
            written: 2 << 20,
            estimated_memory: estimated,
            decoded_memory: decoded,
        }
    }

    #[test]
    fn the_report_lists_the_costliest_kinds_and_the_worst_estimates() {
        let rows = [
            row("audio", "a", 1_000_000, 100, 100),
            row("texture", "t", 3_000_000, 400, 100),
            row("texture", "u", 3_000_000, 100, 100),
            row("video", "v", 5_000_000, 0, 100),
        ];
        assert_eq!(
            report(&rows, 10),
            "kind\tcount\twall_s\tcpu_s\tread_mib\twritten_mib\n\
             texture\t2\t6.0\t6.0\t2.0\t4.0\n\
             video\t1\t5.0\t5.0\t1.0\t2.0\n\
             audio\t1\t1.0\t1.0\t1.0\t2.0\n\
             \n\
             kind\tmeasured\tmean_deviation\tworst_deviation\tworst_subject\n\
             texture\t2\t2.50\t4.00\tt\n\
             audio\t1\t1.00\t1.00\ta\n"
        );
    }

    #[test]
    fn top_limits_each_list() {
        let rows = [row("a", "x", 2, 1, 1), row("b", "y", 1, 1, 1)];
        let text = report(&rows, 1);
        assert_eq!(text.lines().filter(|l| l.starts_with('a')).count(), 2);
        assert!(!text.lines().any(|l| l.starts_with('b')));
    }
}
