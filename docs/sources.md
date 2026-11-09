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

The game data tables have their own source, the `tables` entry of a region: `git` reads a repository of table dumps over HTTP at its current revision, and `dir` reads `<table>.json` files from a local directory. The revision is recorded in the [table identity](#version-identity) of the version, and the hash of each row a converter reads enters that converter's fingerprints.

## Region configuration

A region of `"kind": "dir"` needs only `"directory"`: the files under it are its bundles, named by their relative paths, and their dependencies are read from the external files the bundles name (see [Streaming evaluation](pipeline.md#streaming-evaluation)). Any region can list `"stores"`, the output directories of earlier runs. A bundle with the same declared hash is copied from the first of them that holds it instead of being downloaded.

Regions are defined by the deployer in a JSON configuration file, which is passed to the exporter with `--config`. The exporter contains no addresses, application identifiers or client identities. An entry gives the protocol of the region (`kind`: `cp` or `nuverse`), its platform, its hosts, its address templates, the client identity (application version, application hash, asset version and hash, Unity version, user agent, device model and operating system), the names of its secrets, an optional feed and the network settings. The Unity version is only sent with requests: readers follow the format versions stored in each file. **The configuration never contains secret values.**

```json
{
  "regions": {
    "example": {
      "kind": "nuverse",
      "platform": "ios",
      "hosts": ["cdn-a.example.invalid", "cdn-b.example.invalid"],
      "urls": {
        "version": "https://{host}/{application_version}/{platform}/version",
        "manifest": "https://{host}/{application_version}/{platform}{value}/manifest",
        "bundle": "https://{host}/{application_version}/{download_path}/{bundle}"
      },
      "client": { "application_version": "1.0.0" },
      "secrets": { "key": "SEKAI_MANIFEST_KEY", "iv": "SEKAI_MANIFEST_IV", "hca": "SEKAI_HCA_KEY" },
      "feed": "https://feed.example.invalid/versions.json",
      "tables": {
        "kind": "git",
        "revision": "https://tables.example.invalid/commits/HEAD",
        "table": "https://tables.example.invalid/{revision}/{name}.json"
      },
      "network": { "concurrency": 8, "rate_limit": 20, "retries": 4, "back_off_ms": 500 }
    }
  }
}
```

An address template replaces each `{name}` with its value:

| Template | `cp` | `nuverse` |
|---|---|---|
| `signature` (optional) | none | not used |
| `version` | `application_version`, `application_hash` | `host`, `application_version`, `platform` |
| `manifest` | `profile`, `host_hash`, `asset_version`, `asset_hash`, `platform` | `host`, `application_version`, `platform`, `value` |
| `bundle` | The names of `manifest`, and `bundle` | The names of `manifest`, and `download_path`, `bundle` |

## Version discovery

The client identity is taken from the first of these that is available:

1. the client package given as an `apk` input;
2. the configuration;
3. an external feed of current versions.

A feed is a JSON document with the fields `appVersion`, `appHash`, `assetVersion`, `assetHash` and `multiPlayVersion`, optionally under `production_<platform>`. A candidate is adopted only if the region's version endpoint confirms it and it is newer than the current value. For regions whose distribution paths are keyed by the bundle version, an application version `x.y.z` that the endpoint does not recognize is retried as `x.y.0`. A value set with `force` is a pin: it is not replaced by discovery until it is removed, which allows a rollback. Each region is checked independently, so a failure in one region does not delay the others.

## Version identity

The state of a region has three separate parts, and each decides something different.

| Part | Fields | When it changes |
|---|---|---|
| **Asset identity** | `cp`: asset version, asset hash, profile and asset host hash. `nuverse`: the application version of the distribution paths and the value of the version endpoint | The addresses of the manifest and the bundles change, so the manifest is read again and P1 to P3 run |
| **Table identity** | The table revision: the commit of the dump repository, or the hash of the files of a `dir` source | Only table rows can differ. Stored bundles and their indexes are reused, and only the assets whose rows changed are evaluated again |
| **Client identity** | Application version, application hash and multiplayer version | Recorded with the version, and no asset depends on it. For `cp` it is the key that finds the asset host hash, so the asset identity is read through it |

A committed version is named by its asset identity and table identity together, so an update of the tables alone is a new version that shares every unaffected output with the previous one.

## Declared hashes and content hashes

A manifest row declares a bundle hash (`hash` for `cp`, `crc` for `nuverse`) and other fields, such as a size. These are two different kinds of hash from the SHA-256 that identifies a stored input:

- The **declared hash** decides whether a bundle is fetched again. A bundle whose declared hash matches the last run is not downloaded, and keeps its stored bytes and its index. The other declared fields can change while the declared hash stays the same, so they are recorded but not compared.
- The **content hash** is the SHA-256 the exporter computes while the bytes arrive. Everything after the download identifies the bundle by it.

A declared hash is a label, not a digest of the file. Bundles with the same declared hash can be served with different bytes under the paths of different versions, including different streamed data, and bundles with different declared hashes can hold identical objects. Runs therefore do not repeat a download to compare it, and incremental work follows the content of objects and of the resource entries of their bundles, as described in the [pipeline](pipeline.md#invariants).

## Credentials

Manifests and interface responses are encrypted. The decryption keys, the keys for encrypted CRI audio and video, and the guest account used by the Japanese version are **secrets**.

- Each secret has a name. Its value is read from an environment variable of that name, or from a secret file outside the repository.
- Secret values are never written to logs, reports, the catalog or asset documents. Reports name the secret that was used, not its value.
- The guest account is created once by `sekai-exporter account`, and is then stored as a secret and reused. Runs do not create accounts. A `cp` region that names the secret `secrets.account` and has the addresses `urls.account` and `urls.login` takes its asset version from logging in with that account.
- The configuration holds only the name of each secret. Its value must be obtained by the user from their own client, and is written as hexadecimal.
- `secrets.hca` is optional and names the key of encrypted HCA audio, a 64-bit number written as one to sixteen hexadecimal digits. The audio converter decrypts with it and the subkey stored in each cue sheet. Without it, encrypted audio is reported with the error `missing_key`.

## Network

Each source can be configured with a proxy (HTTP or SOCKS), a concurrency limit, a rate limit, retries with back-off and host rotation. Manifests carry no content digest. The exporter checks the length of each bundle, against the declared size for `nuverse` and against the length of the response for `cp`, whose declared size is not the size of the download, and records its SHA-256.

## The input store

A deployment that keeps raw inputs uses them as a `store` source. Lazy evaluation and serverless deployments then read inputs from that store, so credentials are needed only on the machine that fetches bundles, and earlier versions can be evaluated again. **The input store contains game data and must be private.** It must not be published with the outputs.
