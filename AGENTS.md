# Instructions for coding agents

Read [CONTRIBUTING.md](CONTRIBUTING.md) before changing anything. Its rules apply to every change, and every command under its Checks passes before a change is done. Three points need the most care.

## Keep stored results valid

Every deployment keeps the results of earlier runs and takes them again while their identity is unchanged. The identity holds the version of each form and table converter, the encoder versions, the revision of units and the format versions of the published documents in `sekai-format`. A new value for any of them converts or publishes the affected assets again in every deployment, which can mean all of them.

Change one only when a correct result cannot be reached otherwise, and prefer a change that keeps existing results valid. A converter that learns to read assets it used to fail needs a new revision, not a new version (see `Converter::version` and `Converter::revision`). A pull request that changes any of these values states which one and what is converted again.

## Keep the documentation in both languages and current

Every document has an English and a Chinese version: `name.md` and `name.zh-CN.md`, or both languages in one file such as `docs/glossary.md`. A change to behaviour, a command, an option, a format or a form updates the documents that describe it, in both languages, in the same pull request. A change to the state of a part also updates [the roadmap](docs/roadmap.md). This file is the one exception and is written in English only.

## A new form: research, then specification, then code

1. **Research.** Establish what the asset must contain for the runtime that reads it: how that runtime loads and uses it, and which game objects and tables hold the data. Justify every choice from the source code or format definition that owns it.
2. **Specification.** Write `docs/spec/<form>.md` and its Chinese version as [Specifications](docs/spec/README.md) describes, and settle it before the code.
3. **Code.** Implement the converter behind its own feature, as the specification states, with tests on synthetic inputs.
