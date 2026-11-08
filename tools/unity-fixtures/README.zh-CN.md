# unity-fixtures

一个 Unity 2022.3.62f2 工程，用于构建 `crates/sekai-unity/tests/fixtures` 中的样例资源包。这些资源包只含本工程创建的对象，因而可以随仓库分发。

```bash
Unity -batchmode -quit -nographics -projectPath tools/unity-fixtures -buildTarget Android \
      -executeMethod BuildFixtures.Build -fixtureOutput crates/sekai-unity/tests/fixtures
```

工程构建 `fixture_data` 与 `fixture_scene` 两个资源包，各有不压缩、LZ4 与 LZMA 三种形式。它们覆盖对象文档规范中的每一种值，以及 `[SerializeReference]` 字段、超过内联上限的字节载荷、非有效 UTF-8 的字符串，以及文件内与指向其他资源包的引用。

`BuildTextureFixtures.Build` 把一张程序生成的图按每种贴图格式各构建一个资源包；[compare-texture](../compare-texture/README.zh-CN.md) 再把这些资源包转成 `sekai-codec` 的样例。

三种形式所含的序列化文件相同，因此其对象文档完全一致。重新构建之后，以 `UPDATE_EXPECTED=1 cargo test -p sekai-unity --test fixtures` 重新生成期望文档，并用 [compare-unitypy](../compare-unitypy/README.zh-CN.md) 核对。生成的资产在每次构建时重新创建，因此重新构建可能改变 path ID 与哈希。
