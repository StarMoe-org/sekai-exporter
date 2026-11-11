# Performance

[中文](performance.zh-CN.md)

Performance is a property of the evaluator, not of converters. A converter is a pure function from an object view and a context to asset bytes. Caching, incremental evaluation, sharing, laziness, parallelism, fused encoding and memory control all live in the evaluator, and no converter contains threads, caches or input and output. Optimizations are therefore kept in one place, contributors who write converters do not need to know them, and each optimization can be turned off to compare against evaluation without it.

The check that holds this together: **optimized evaluation and plain evaluation produce identical bytes.** Plain evaluation runs on one thread, reads and writes no derivation records and encodes nothing fused. Tests on synthetic bundles compare it with evaluation on many threads, in segments, lazily, and with a bounded block cache or none, and continuous integration compares the store a `wasm32` build writes with the store of the native build byte for byte.

## Order of concerns

Work is reduced in this order, and each step is applied before the next is considered.

| Step | Method | Where |
|---|---|---|
| **Avoid work** | Reuse any derivation whose fingerprint already has an output; share downloads, conversions and storage across regions by content hash; evaluate only the bindings that are requested; treat the index of a bundle as a derivation that is reused while the bundle is unchanged; keep the file of an encoding whose decoded content was encoded before | Evaluator and store |
| **Read less** | The index stage extracts references, names and content hashes only, and skips large arrays (pixels, vertices, audio blocks) by length; payloads are read when a conversion needs them; external streams are read by byte range | `sekai-unity` |
| **Copy less** | Decompressed blocks are kept in a cache bounded by the memory budget, and readers borrow slices from it; object documents are written while the object is read, without a value tree; converters read objects as value trees, and typed readers that read directly from the bytes are added for classes where measurement shows the value tree is costly | `sekai-unity` |
| **Run in parallel** | One scheduler, described below | `sekai-pipeline` |

## Reading

**Layout compilation.** A type tree is compiled into a flat reading program: read a fixed-size value, skip a fixed number of bytes, align, loop over an array, read a PPtr, read a string. Each type tree of a file is compiled once, when the file is opened. The reference extractor used by the index, the object document writer and the typed readers are interpretations of the same program, so there is one implementation of each layout. Layout tables for objects stored without type trees compile to the same kind of program.

**Block cache.** UnityFS stores data in LZ4 or LZMA blocks. Decompressed blocks are cached under the key `(bundle content hash, block index)`, and the cache counts against the memory budget. Reading an object decompresses only the blocks that contain it, and a block that several readers miss at once is decompressed once.

**Files.** A bundle in the store is read from its file at an offset, without moving a cursor, so threads share one handle. The most recently read files stay open between reads, up to a fixed number.

**Bundle indexes.** The index of a bundle, with the digest of the stream or resource slice each object reads, is a function of the bundle bytes alone, so it is stored in the catalog under their SHA-256. Opening a version reads the stored index of every bundle it has seen before and parses only the new ones. A bundle is parsed again when an object or a resource of it is read, only the most recently used bundles stay parsed, and a bundle that several readers open at once is parsed once.

## Scheduling

- Evaluation streams over the manifest in segments: consecutive units in name order, each unit a bundle with the bundles it depends on, bounded by a number of units and by the declared bytes of their bundles. Two segments convert at once while the bundles of the next one are opened and indexed, and a bundle is let go once no later unit needs it.
- One scheduler runs on standard library threads. Its jobs are the derivations of the segments and the indexing of their bundles.
- It manages three resources, each configured by a single number: CPU slots (by default the number of logical CPUs), a memory budget (by default half of physical memory), and the number of concurrent connections to each remote endpoint. There are no other pool settings.
- Each derivation declares a cost class and an estimate of its peak memory, computed from object headers: the stored image data and a multiple of the decoded size for a texture, a multiple of the stored size of the cue sheet for audio, and a multiple of the stored size otherwise. A job that does not fit in the remaining budget waits, and lets a bounded number of later jobs that fit start ahead of it; a job larger than the whole budget runs alone.
- A job takes CPU slots for each part of its work, and returns them before its files are written to the store. No other thread pool exists.
- Within a segment, the jobs that read the same bundles are grouped. Groups start in decreasing order of their longest job, and the jobs of a group longest first, which keeps the blocks a group reads in the cache and shortens the wait for the last jobs.
- Network transfer is separate from computation. A download is hashed and deobfuscated while it streams into a temporary file in its directory of the input store, and is placed there by a hard link. Requests to one host share one HTTP/2 connection, and a bundle that an earlier run holds is linked rather than downloaded. Bundles are fetched while evaluation runs, in the order it opens them, and opening a bundle waits until it has arrived, so the first segments convert while later bundles are still on their way. Computation reads only from the store.

## Hot paths

| Work | Method |
|---|---|
| Texture decoding (ASTC, ETC2, BC) | One image per job; the images of a segment decode in parallel |
| Image encoding | **Fused**: one decoded surface is encoded as the canonical PNG and, for the storage view, as lossy WebP in the same job, without decoding again or queueing the surface |
| Audio | HCA frames are decoded and passed to the MP3 encoder one at a time, so no stream is held decoded whole; members with the same stream and key are encoded once |
| Video | Each movie is encoded by one `ffmpeg` process with a fixed thread count, which reads the video stream from its standard input |
| Encodings | Each encoding is recorded under a fingerprint of its encoder and its decoded content; a conversion that decodes the same content again keeps the stored file without encoding |
| Object documents | Written as JSON directly by the compiled reading program, without building a value tree |
| Store writes | Content-addressed; content that already exists is skipped. A file is written next to its destination and renamed into place, and a local store materializes path trees with hard links |
| Publication | Plans, indexes, path trees and the storage view are built in parallel; the check that every member is in the store lists each store directory once instead of opening every file |
| Catalog | One redb file with batched write transactions; read transactions see a snapshot and do not wait for writes |

Fused encoding does not change fingerprints: each output is the same bytes it would be without fusion, and a test compares fused evaluation with plain evaluation.

## Measurement

- Each derivation records its wall time, CPU time, bytes read and written, estimated peak memory and the decoded size of its members in the catalog. The cost of a derivation is its latest measurement. `sekai-exporter stats` lists the most expensive derivation kinds and the converters whose memory estimates are furthest from that decoded size. An estimate also covers working copies and the encoder process, so some distance is expected. Optimization starts from this record.
- Each run writes `metrics.jsonl` next to its output, one JSON document per line, and shows a status line while it works: the CPU and memory use of the process, of its child processes and of the machine, sampled every second, and a span for each stage and derivation with its wall time, CPU time and bytes moved. The evaluator reports through a `Recorder` and reads no clock itself, so measurement cannot influence any output.
- Benchmarks over real game data are run locally by whoever has the data, and their results are not part of this repository.
