# Specifications

Each asset form and each published document format has a specification in this directory. A specification is written before the converter or writer that implements it, and states:

1. **Runtime**: the consumer the form is for.
2. **Contents**: what the asset contains, field by field or file by file.
3. **Sources**: the objects and game data tables the asset is built from, and the rules that select and combine them.
4. **Container**: the file format, its version and every encoding parameter.
5. **Correctness**: why the result is correct for that runtime, and how it is checked.
6. **Version**: the version of the form. A change to any of the above changes the version, and the version enters the fingerprint of every asset of the form.

Specifications are written in English.
