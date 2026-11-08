# Performance

[中文](performance.zh-CN.md)

Performance is a property of the evaluator, not of converters. A converter is a pure function from an object view and a context to asset bytes. Caching, incremental evaluation, sharing, laziness, parallelism, fused encoding and memory control all live in the evaluator, and no converter contains threads, caches or input and output. Optimizations are therefore kept in one place, contributors who write converters do not need to know them, and each optimization can be turned off to compare against evaluation without it.

The check that holds this together: **optimized evaluation and plain evaluation produce identical bytes.** Plain evaluation runs on one thread with no derivation cache and no fused encoding. Continuous integration compares one thread against many and a bounded block cache against none. Comparing optimized against plain evaluation, and native against `wasm32`, is planned for when the evaluator and the converters exist.

## Order of concerns

Work is reduced in this order, and each step is applied before the next is considered.

| Step | Method | Where |
|---|---|---|
| **Avoid work** | Reuse any derivation whose fingerprint already has an output; share downloads, conversions and storage across regions by content hash; evaluate only the bindings that are requested; treat the index of a bundle as a derivation that is reused while the bundle is unchanged | Evaluator and store |
| **Read less** | The index stage extracts references, names and content hashes only, and skips large arrays (pixels, vertices, audio blocks) by length; payloads are read when a conversion needs them; external streams are read by byte range | `sekai-unity` |
| **Copy less** | Decompressed blocks are kept in a cache bounded by the memory budget, and readers borrow slices from it; object documents are written while the object is read, without a value tree; converters read objects as value trees, and typed readers that read directly from the bytes are added for classes where measurement shows the value tree is costly | `sekai-unity` |
| **Run in parallel** | One scheduler, described below | `sekai-pipeline` |

## Reading

**Layout compilation.** A type tree is compiled into a flat reading program: read a fixed-size value, skip a fixed number of bytes, align, loop over an array, read a PPtr, read a string. Each type tree of a file is compiled once, when the file is opened. The reference extractor used by the index, the object document writer and the typed readers are interpretations of the same program, so there is one implementation of each layout. Layout tables for objects stored without type trees compile to the same kind of program.

**Block cache.** UnityFS stores data in LZ4 or LZMA blocks. Decompressed blocks are cached under the key `(bundle content hash, block index)`, and the cache counts against the memory budget. Reading an object decompresses only the blocks that contain it.

## Scheduling

- One scheduler runs on standard library threads, and its tasks are the nodes of the derivation graph.
- It manages three resources, each configured by a single number: CPU slots (by default the number of logical CPUs), a memory budget (by default half of physical memory), and the number of concurrent connections to each remote endpoint. There are no other pool settings.
- Each derivation declares a cost class and an estimate of its peak memory. Estimates are computed from object headers: width × height × 4 for a texture, samples × channels × 4 for audio, and a multiple of the payload length otherwise. A task that does not fit in the remaining budget waits; a task larger than the whole budget runs alone.
- Tasks that can be divided internally, such as decoding a large texture or encoding long audio, are split into subtasks that return to the same scheduler. No other thread pool exists.
- Tasks run in topological order and, within one level, in decreasing order of estimated time, which shortens the wait for the last tasks.
- Network transfer is separate from computation. A download is hashed and deobfuscated while it streams, and written directly to the store without a temporary file. Computation reads only from the store.

## Hot paths

| Work | Method |
|---|---|
| Texture decoding (ASTC, ETC2, BC) | Parallel over blocks of the image, with portable SIMD in the inner loops |
| Image encoding | **Fused**: one decoded surface is encoded in every format the run needs (the lossless canonical image and the format a view requires) in the same task, without decoding again or queueing the surface |
| Audio | HCA is decoded once, and the samples are passed to every encoder the run needs |
| Video | VP9 streams are written into MP4 without re-encoding |
| Object documents | Written as JSON directly by the compiled reading program, without building a value tree |
| Store writes | Content-addressed; content that already exists is skipped. A local store materializes path trees with hard links |
| Catalog | SQLite in WAL mode with batched transactions; read-only queries use separate connections and do not wait for writes |

Fused encoding does not change fingerprints: each output is the same bytes it would be without fusion, and comparing fused and unfused evaluation in continuous integration is planned together with fused encoding.

## Measurement

- Each derivation records its wall time, CPU time, bytes read and written, and estimated and actual peak memory in the catalog. `sekai-exporter stats` lists the most expensive derivation kinds and the converters whose memory estimates are least accurate. Optimization starts from this record.
- The hot paths have benchmarks on synthetic inputs. Regression checks use instruction counts, which are stable on shared machines, rather than wall time.
- Benchmarks over real game data are run locally by whoever has the data, and their results are not part of this repository.
