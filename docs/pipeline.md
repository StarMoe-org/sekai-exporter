# Pipeline

[中文](pipeline.zh-CN.md)

The pipeline evaluates the [derivation graph](architecture.md) bundle by bundle, with the streaming evaluator described below. One run processes the current version of every configured region in a single process, with one catalog and one derivation cache shared by all regions.

## Stages

| Stage | Work | Output |
|---|---|---|
| P0 Plan | Read the [asset identity and table identity](sources.md#version-identity) of each region | Nothing to do if both match the last complete run. If only the table identity changed, P1 to P3 are skipped and the index of the asset identity is reused |
| P1 Manifest | Download and parse the manifest; select bundles by name, with their dependencies | The bundle list with declared hashes, sizes and dependencies |
| P2 Fetch | Download each selected bundle, deobfuscate it and hash it while it streams, and write it to the store | Bundle inputs, identified by content hash |
| P3 Index | For each bundle, when the first unit that needs it opens it, read every object's fields, skipping large arrays by length; record class, name, container path, content hash and references. The index is stored by the content hash of the bundle and reused | Object rows and reference edges |
| P4 Plan units | For each bundle, in name order and in segments (see [Streaming evaluation](#streaming-evaluation)): resolve external files and `m_Script` within its window; each converter selects the root objects of the bundle and computes closures over the reference edges and the further inputs it declares, within the window; names are assigned from the bundle (see [Identity](architecture.md#identity)); fingerprints are computed | Derivations to evaluate; derivations whose fingerprint already has an output are reused without reading any bytes |
| P5 Convert | Evaluate the convert and transform derivations of a segment under the scheduler | Asset documents and file bytes |
| P6 Write | Write bytes to the store, skipping content that already exists; record derivations in the catalog | Store objects and catalog rows |
| P7 Tables | Build the assets of the [table stage](#table-stage) | Asset documents and file bytes |
| P8 Views | Each enabled view produces bindings; views that need other bytes produce transform derivations | Bindings |
| P9 Commit | Verify that every bound object exists in the store with its recorded hash, then advance the version pointer | A committed version, with its asset errors listed |

## Invariants

- **Increments follow object content, not bundles.** A bundle that is rebuilt without changing its objects leaves every object hash, and therefore every asset fingerprint, unchanged. P2 and P3 run only for bundles whose [declared hash](sources.md#declared-hashes-and-content-hashes) changed; the index of an unchanged bundle is reused.
- **Every object is accounted for.** `ledger.tsv` lists each bundle with the number of its objects that are roots, read by a conversion, of a class or script declared to carry no asset content, or unknown, and of its resource files read. A bundle's account is settled after the last unit whose window holds it, when no later conversion can read it. A bundle is complete when nothing is unknown and every resource file was read.
- **A conversion reads only its closure.** Reading any other object fails the asset with the error `undeclared_read`, so everything an output depends on is in its fingerprint.
- **Fingerprints contain everything that determines the bytes:** the converter name and version, the content hash of each object in the closure, the content hash of each resource entry in the bundles of those objects, and the encoder name, version and parameters. Assets built from tables have the fingerprint described under [Table stage](#table-stage). Configuration does not change output paths, formats or encoder parameters; those are defined by converters and views.
- **Asset errors and run failures are different things.** An asset error is the result of evaluating an asset: a named error that is as deterministic as a successful output. It does not affect other assets, it does not stop the version from being committed, and it is listed with the version. Because it has no successful output, the next run evaluates it again. Missing members, such as a bundle that the asset list names but the source does not serve, are asset errors. A run failure is a failed download, a failed write or a crash. Only run failures stop a commit.
- **The order of work does not affect output.** Each output depends only on its inputs, and the catalog is written in key order at the end of each batch. Store contents and the catalog contents sorted by primary key are independent of thread count and scheduling.
- **A commit is all or nothing per version.** P9 advances the pointer only after verification. When verification fails, the stage that failed is reported by its fixed name and the previous pointer stays in place.

## Streaming evaluation

The streaming evaluator evaluates a version from its manifest, one bundle at a time. No step needs the whole version at once.

- **Units.** A unit is one bundle of the manifest with its window: the bundle and every bundle it depends on, directly or through others. The roots of a unit are the root objects of its bundle. They are named from that bundle alone, and their closures, declared inputs and script references are followed only inside the window.
- **References outside the window.** A reference in the closure of a root to a file of a bundle outside the window makes the asset fail with the error `outside_dependencies`, whose message names the referring bundle and object and the external file. A script reference outside the window does not resolve. Every such reference is listed with the result. References to engine files that no bundle holds stay out of closures.
- **Dependencies.** A source whose manifest lists no dependencies, such as a directory, has them read from the bundles: a bundle depends on each bundle that holds a file its serialized files name as an external file. When several bundles hold a file of the same name, the first in name order holds it.
- **Segments.** Units run in manifest order, in segments of consecutive units, bounded by a number of units and by the declared size of the bundles of their windows. The bundles of a segment are opened together and its derivations run under the scheduler. A bundle stays open, indexed and parsed until the last unit whose window holds it, which the manifest tells in advance. Parsed bundles are kept within an estimate of their memory, and stored indexes are kept in memory only while a later unit needs them.
- **Results.** Every closure stays inside its own window, so documents, member files, fingerprints, names and ledgers do not depend on how units are grouped into segments or on the number of threads. Assets of different bundles that receive the same name are left unnamed. Publishing follows the last unit.
- **Errors.** Bundles that cannot be read, opened or indexed, bundles that share a name, and dependencies that the manifest does not list are reported by bundle name. The units that need them still run, and references that reach them fail as described above.

## Table stage

Some assets, such as [episodes](features.md), have no root object: each is built from rows of the game data tables and from assets the version already holds. After the last unit, each table converter reads the tables it needs and lists its assets, and each asset is built.

- **Inputs.** A table converter sees the tables, the assets of each bundle with their forms, the document of an asset and the bytes of its members. It reads no object of a bundle.
- **References.** An asset that a row needs and the version does not hold is recorded in the asset being built. A run over a selection of bundles therefore gives assets that record what the selection left out.
- **Failures.** A table that cannot be read or parsed is a run failure. An asset that cannot be built is an asset error.
- **Fingerprints.** These assets are built in every run and no derivation is recorded. The fingerprint is the hash of the form and its version, the name, the role and content hash of each member, the references and any error, so it changes exactly when the output changes.

## Regions

All regions run in one process. P0 to P3 run for every region in parallel. In P4 and P5 a fingerprint computed for any region is found by every other region, so content shared between regions is converted once. The download stage reuses a bundle already fetched for another region when both regions declare the same hash.

Game data tables are separate for each region. A converter's fingerprint covers only the rows it reads, so an asset whose rows are identical in two regions is still shared even when the tables differ elsewhere.

## The client source

Assets inside the client package go through the same pipeline. In P1 the manifest is replaced by the list of player data files in the package. In P3, objects without type trees are read with the field layout tables in `sekai-layouts`. Every object must be read exactly to its end; an object that is not read exactly is written as raw bytes with a named error.
