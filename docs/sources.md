# Sources

[中文](sources.zh-CN.md)

A source supplies inputs: the current version of a region, its manifest, and bundle bytes by name. Bytes become inputs as soon as they arrive, identified by their SHA-256. Every later derivation knows inputs only by hash and does not depend on where the bytes came from.

## Kinds of source

| Source | Supplies | Used for |
|---|---|---|
| `cp` | Versions, manifests and bundles from the distribution service used by the Japanese and English versions | `jp`, `en` |
| `nuverse` | Versions, manifests and bundles from the distribution service used by the Chinese, Taiwanese and Korean versions | `cn`, `tw`, `kr` |
| `dir` | Bundles already present in a local directory | Offline runs and tests |
| `apk` | Player data from a client package, and the client identity it contains | The `client` source |
| `store` | Inputs kept by the deployment in its own private store | Lazy evaluation, serverless deployments and history |

A region can list several sources in order, for example `store` first and `cp` when the store does not have the bundle.

The game data tables have their own sources: `git`, which reads a repository of table dumps at a given commit and is the default, and `api`, which reads the game's interface and requires an account. The commit or interface version is recorded in the [table identity](#version-identity) of the version, and the hash of each row a converter reads enters that converter's fingerprints.

## The region table

The hosts, application identifiers, channels and platforms of each region, together with the client identity (application version, application hash, user agent and device model), are kept in a data file in this repository. The configuration can override any entry. These values are required for the exporter to work at all, and anyone with an installed client can read them. **The region table never contains keys.**

## Version discovery

The client identity is taken from the first of these that is available:

1. the client package given as an `apk` input;
2. the configuration;
3. an external feed of current versions.

A candidate is adopted only if the region's version endpoint confirms it and it is newer than the current value. For regions whose distribution paths are keyed by the bundle version, an application version `x.y.z` that the endpoint does not recognize is retried as `x.y.0`. A value set with `force` is a pin: it is not replaced by discovery until it is removed, which allows a rollback. Each region is checked independently, so a failure in one region does not delay the others.

## Version identity

The state of a region has three separate parts, and each decides something different.

| Part | Fields | When it changes |
|---|---|---|
| **Asset identity** | `cp`: asset version, asset hash, profile and asset host hash. `nuverse`: the value of the version endpoint | The addresses of the manifest and the bundles change, so the manifest is read again and P1 to P3 run |
| **Table identity** | The data version the game reports, and the table revision: the commit of the dump repository, or the interface version for the `api` source | Only table rows can differ. P1 to P3 are skipped, the index of the asset identity is reused, and only the assets whose rows changed are evaluated again |
| **Client identity** | Application version, application hash and multiplayer version | Recorded with the version, and no asset depends on it. For `cp` it is the key that finds the asset host hash, so the asset identity is read through it |

The data version and the table revision are separate fields because they come from different places: the game assigns one, and the repository that holds the dump assigns the other. A change of either one changes the table identity.

A committed version is named by its asset identity and table identity together, so an update of the tables alone is a new version that shares every unaffected output with the previous one.

## Declared hashes and content hashes

A manifest row declares a bundle hash (`hash` for `cp`, `crc` for `nuverse`), and for `cp` also a `crc` and a size. These are two different kinds of hash from the SHA-256 that identifies a stored input:

- The **declared hash** decides whether a bundle is fetched again. A bundle whose declared hash matches the last run is not downloaded, and keeps its stored bytes and its index. The other declared fields can change while the declared hash stays the same, so they are recorded but not compared.
- The **content hash** is the SHA-256 the exporter computes while the bytes arrive. Everything after the download identifies the bundle by it.

A declared hash is a label, not a digest of the file. Bundles with the same declared hash can be served with different bytes under the paths of different versions, including different streamed data, and bundles with different declared hashes can hold identical objects. Runs therefore do not repeat a download to compare it, and incremental work follows the content of objects and of the resource ranges they read, as described in the [pipeline](pipeline.md#invariants).

## Credentials

Manifests and interface responses are encrypted. The decryption keys, the keys for encrypted CRI audio and video, and the guest account used by the Japanese version are **secrets**.

- Each secret has a name. Its value is read from an environment variable of that name, or from a secret file outside the repository.
- Secret values are never written to logs, reports, the catalog or asset documents. Reports name the secret that was used, not its value.
- The guest account is created once by a separate command that the deployer runs, and is then stored as a secret and reused. Runs do not create accounts.
- The example configuration leaves every secret empty and states that its value must be obtained by the user from their own client.

## Network

Each source can be configured with a proxy (HTTP or SOCKS), a concurrency limit, a rate limit, retries with back-off and host rotation. Manifests carry no content digest and their declared size is not the size of the download, so the exporter checks each bundle against the length of its response and records its SHA-256.

## The input store

A deployment that keeps raw inputs uses them as a `store` source. Lazy evaluation and serverless deployments then read inputs from that store, so credentials are needed only on the machine that fetches bundles, and earlier versions can be evaluated again. **The input store contains game data and must be private.** It must not be published with the outputs.
