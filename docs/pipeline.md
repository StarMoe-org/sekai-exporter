# Pipeline

[中文](pipeline.zh-CN.md)

The pipeline is the eager evaluator of the [derivation graph](architecture.md). One run processes the current version of every configured region in a single process, with one catalog and one derivation cache shared by all regions.

## Stages

| Stage | Work | Output |
|---|---|---|
| P0 Plan | Read the current asset version, application version and game data revision of each region | Nothing to do if all of them match the last complete run |
| P1 Manifest | Download and parse the manifest; select bundles by name | The bundle list with declared hashes, sizes and dependencies |
| P2 Fetch | Download each selected bundle, deobfuscate it and hash it while it streams, and write it to the store | Bundle inputs, identified by content hash |
| P3 Index | For each bundle, read every object's fields, skipping large arrays by length; record class, name, container path, content hash and references | Object rows and reference edges |
| P3b Resolve | After every bundle is indexed: resolve external files to bundles, `m_Script` to class names, and write object documents | A fully resolved object graph for the version |
| **Barrier** | Asset closures and names depend on the whole version | |
| P4 Plan assets | Each converter selects its root objects and computes closures over the reference edges; names are assigned (see [Identity](architecture.md#identity)); fingerprints are computed | Derivations to evaluate; derivations whose fingerprint already has an output are reused without reading any bytes |
| P5 Convert | Evaluate convert and transform derivations under the scheduler | Asset documents and file bytes |
| P6 Write | Write bytes to the store, skipping content that already exists; record derivations in the catalog | Store objects and catalog rows |
| P7 Views | Each enabled view produces bindings; views that need other bytes produce transform derivations | Bindings |
| P8 Commit | Verify that every bound object exists in the store with its recorded hash, then advance the version pointer | A complete or partial version |

## Invariants

- **Increments follow object content, not bundles.** A bundle that is rebuilt without changing its objects leaves every object hash, and therefore every asset fingerprint, unchanged. P2 and P3 run only for bundles whose declared hash changed; the index of an unchanged bundle is reused.
- **Fingerprints contain everything that determines the bytes:** the converter name and version, the content hash of each object in the closure, the hash of each table row the converter reads, and the encoder name, version and parameters. Configuration does not change output paths, formats or encoder parameters; those are defined by converters and views.
- **Failures are recorded per asset.** A failed asset carries a named error and does not affect other assets. Because it has no successful output, the next run evaluates it again. Missing members, such as a referenced bundle that cannot be downloaded, are reported the same way.
- **The order of work does not affect output.** Each output depends only on its inputs, and the catalog is written in key order at the end of each batch. Store contents and the catalog contents sorted by primary key are independent of thread count and scheduling.
- **A commit is all or nothing per version.** P8 advances the pointer only after verification. When verification fails, the stage that failed is reported by its fixed name and the previous pointer stays in place.

## Regions

All regions run in one process. P0 to P3 run for every region in parallel. In P4 a fingerprint computed for any region is found by every other region, so content shared between regions is converted once. The download stage reuses a bundle already fetched for another region when both regions declare the same hash.

Game data tables are separate for each region. A converter's fingerprint covers only the rows it reads, so an asset whose rows are identical in two regions is still shared even when the tables differ elsewhere.

## The client source

Assets inside the client package go through the same pipeline. In P1 the manifest is replaced by the list of player data files in the package. In P3, objects without type trees are read with the field layout tables in `sekai-layouts`. Every object must be read exactly to its end; an object that is not read exactly is written as raw bytes with a named error.
