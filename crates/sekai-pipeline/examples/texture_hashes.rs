//! Converts every texture of the bundles in some directories and prints one line per texture.
//!
//! Usage: `texture_hashes <bundle directory>...`
//!
//! All bundles are opened as one version. Each line is `<bundle>\t<path ID>\t<result>`, where the
//! result is the SHA-256 of the image's RGBA bytes (rows from top to bottom), `none` when the
//! texture holds no image, or `error <code>: <message>`. Bundles that cannot be opened are
//! printed as `<bundle>\t-\terror <code>: <message>`. Lines are sorted.

use std::sync::Arc;

use sekai_convert::image::Texture;
use sekai_convert::{Content, Converter};
use sekai_game::obfuscation::deobfuscate;
use sekai_pipeline::{BundleInput, Version};
use sekai_unity::BoundedBlockCache;
use sha2::{Digest, Sha256};

fn main() {
    let mut inputs = Vec::new();
    let mut lines = Vec::new();
    for dir in std::env::args().skip(1) {
        let mut paths: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let mut bytes = std::fs::read(&path).unwrap();
            if deobfuscate(&mut bytes).is_err() {
                lines.push(format!("{name}\t-\terror obfuscation: unknown header"));
                continue;
            }
            let sha256 = Sha256::digest(&bytes).into();
            inputs.push(BundleInput {
                name,
                source: Arc::new(Arc::<[u8]>::from(bytes)),
                sha256,
            });
        }
    }
    let cache = BoundedBlockCache::new(256 << 20);
    let (version, errors) = Version::open(inputs, &cache);
    for e in errors {
        lines.push(format!("{}\t-\terror {}: {}", e.name, e.code, e.message));
    }
    let converter = Texture;
    for (id, object, paths) in version.objects() {
        if !converter.is_root(object, paths) {
            continue;
        }
        let result = match converter.convert(id, &version) {
            Ok(None) => "none".to_owned(),
            Ok(Some(asset)) => match &asset.members[0].content {
                Content::Image(surface) => hex(&Sha256::digest(surface.pixels())),
                _ => unreachable!("the texture converter produces an image"),
            },
            Err(e) => format!("error {e}"),
        };
        let bundle = &version.index().bundles()[id.bundle].name;
        lines.push(format!("{bundle}\t{}\t{result}", id.path_id));
    }
    lines.sort();
    for line in lines {
        println!("{line}");
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
