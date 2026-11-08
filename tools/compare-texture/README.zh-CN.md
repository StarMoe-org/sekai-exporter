# compare-texture

[English](README.md)

将 `sekai-codec` 的贴图解码器与参考解码器相比对。脚本通过该 crate 的 `decode_stream` 示例程序调用解码器。

```bash
cargo build --release -p sekai-codec --features texture --example decode_stream
pip install UnityPy==1.25.4
```

| 脚本 | 作用 |
|---|---|
| `fuzz_astc.py <decode_stream> [块数]` | 用解码器与 ARM astcenc 分别解码各块尺寸的随机块。像素必须相等；解码器拒绝的块必须是 astcenc 解为错误色的块。 |
| `compare.py <decode_stream> <资源包目录>...` | 按格式统计资源包中的贴图，并将每张不重复的图与参考相比对。存在差异或被拒绝的图时，退出状态为 1。 |
| `make_fixtures.py <资源包目录> <输出目录>` | 由 [unity-fixtures](../unity-fixtures/README.zh-CN.md) 中 `BuildTextureFixtures.Build` 构建的资源包生成 `crates/sekai-codec/tests/fixtures/textures` 中的样例。 |

ASTC 的参考为 astcenc，ETC 的参考为 texture2ddecoder。未压缩格式与其字节的直接展开相比对。Alpha8 展开为黑色加存储的 alpha，即 Unity 从该格式读出的 32 位像素。

持续集成运行 `fuzz_astc.py`，样例由测试套件检查。
