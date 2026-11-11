# Contributing

[中文](CONTRIBUTING.zh-CN.md)

Contributions are welcome, from fixes to whole new asset forms. Start with [Objects and assets](docs/objects-and-assets.md); it explains how the project is built and where a change belongs.

## Where a change goes

| You want | Add or change |
|---|---|
| a new asset form | a converter module behind its own feature, plus its specification in `docs/spec/` |
| a different layout of existing assets | a view module behind its own feature |
| support for a format variant the reader rejects | the unpacking or decoding crate, with a synthetic file that exercises it |
| a fix | the module that is wrong, with a test that fails before the fix |

## Rules

- **No game data in the repository.** That covers bundles, extracted images, audio, models, tables and decompiled code, including in tests, fixtures and issue attachments. Tests build their inputs synthetically, or use files that are explicitly redistributable.
- **Specification first.** A new asset form comes with a document that says what is exported, from which objects and tables, in which format, and why the result is correct.
- **Fail loudly.** If a converter cannot produce a correct asset, it reports a named error for that asset. It never writes a guessed value.
- **No `#[cfg]` inside function bodies.** Features gate whole modules or optional dependencies.
- **Deterministic output.** The same input gives the same bytes, whatever the thread count.
- **No `unsafe` code.** The workspace forbids `unsafe_code`.

## Checks

Before opening a pull request, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
cargo hack check --workspace --each-feature
```

The last command builds every feature on its own, so a feature cannot depend on another one by accident.

## Commits

Use English [Conventional Commits](https://www.conventionalcommits.org/) with a lowercase type, describing the change and its visible effect, for example `feat(export): write sprite atlases as separate images`.

## License

By contributing you agree that your contribution is licensed under the MIT License of this repository.
