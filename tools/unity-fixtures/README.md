# unity-fixtures

A Unity 2022.3.62f2 project that builds the fixture bundles in `crates/sekai-unity/tests/fixtures`. The bundles contain only objects that this project creates, so they can be distributed with the repository.

```bash
Unity -batchmode -quit -nographics -projectPath tools/unity-fixtures -buildTarget Android \
      -executeMethod BuildFixtures.Build -fixtureOutput crates/sekai-unity/tests/fixtures
```

Two bundles are built, `fixture_data` and `fixture_scene`, each uncompressed, with LZ4 and with LZMA. They cover every value kind of the object document specification, `[SerializeReference]` fields, a byte payload above the inline limit, a string that is not valid UTF-8, and references within a file and to another bundle.

`BuildTextureFixtures.Build` builds one bundle per texture format from a generated image; [compare-texture](../compare-texture/README.md) turns those bundles into the fixtures of `sekai-codec`.

The three variants contain the same serialized files, so their object documents are identical. After rebuilding, regenerate the expected documents with `UPDATE_EXPECTED=1 cargo test -p sekai-unity --test fixtures` and check them with [compare-unitypy](../compare-unitypy/README.md). The generated assets are created anew on every build, so a rebuild can change path IDs and hashes.
